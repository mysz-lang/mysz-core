use crate::backend::clback;
use crate::backend::llvmback::LlvmBackend;
use crate::ir::irgen::IRGen;
use crate::ir::tac::Instruction;
use crate::lex::lexer::Lexer;
use crate::parse::parser::Parser as myszparser;
use crate::parse::parsing::{Expr, ExprKind, Identifier, Literal, Parameter, Program, Stmt, Type};
use crate::semantics::analyser::{Analyser, AnalyserError};
use crate::semantics::analysis::FunctionSignature;
use crate::utils::ats::{ATEntry, ATFile, ATInfo, ExportInfo, ImportInfo, ParsedAT, SymbolType};
use crate::utils::ctx::{CompilerCtx, CompilerTarget};
use clap::builder::OsStr;
use cranelift::codegen::Context as clContext;
use cranelift_frontend::FunctionBuilderContext as clFunctionBuilderContext;
use inkwell::OptimizationLevel;
use inkwell::context::Context as inkContext;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use serde_derive::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

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
        AnalyserError::OverDefinitionError { location, message } => (location, message),
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

fn build_module_registry(ats: &[ATInfo]) -> HashMap<String, HashMap<Vec<String>, PathBuf>> {
    let mut registry = HashMap::new();
    for at in ats {
        let mut file_map = HashMap::new();
        for file in &at.files {
            file_map.insert(file.module_path.clone(), file.path.clone());
        }
        registry.insert(at.name.clone(), file_map);
    }
    registry
}

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
        "^",
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
            "^",
            message
        ));
    }

    error_messages.join("\n")
}

fn read_and_lex_file(
    file_path: &Path,
    json_output: bool,
) -> Result<(String, Vec<crate::lex::lexing::Token>), String> {
    let mut file = File::open(file_path).map_err(|e| {
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
    file.read_to_string(&mut source).map_err(|e| {
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

fn collect_public_symbols_from_file(file_path: &Path) -> Result<Vec<String>, String> {
    let (_, tokens) = read_and_lex_file(file_path, false)?;
    let mut parser = myszparser::new(tokens);
    parser.parse();

    if !parser.parser_errs.is_empty() {
        let first = &parser.parser_errs[0];
        return Err(format!(
            "Parser error in {}: {}",
            file_path.display(),
            first.message
        ));
    }

    let mut symbols = Vec::new();
    for stmt in &parser.ast.statements {
        match stmt {
            Stmt::Function { name, public, .. } => {
                if *public {
                    symbols.push(name.value.clone());
                }
            }
            Stmt::Struct { name, .. } => symbols.push(name.value.clone()),
            Stmt::Enum { name, .. } => symbols.push(name.value.clone()),
            Stmt::Constant { name, .. } => symbols.push(name.value.clone()),
            _ => {}
        }
    }
    Ok(symbols)
}

fn collect_symbols_from_at(at: &ATInfo) -> Result<Vec<(String, PathBuf)>, String> {
    let mut symbols = Vec::new();
    for file in &at.files {
        let file_symbols = collect_public_symbols_from_file(&file.path)?;
        for sym in file_symbols {
            symbols.push((sym, file.path.clone()));
        }
    }
    Ok(symbols)
}

#[allow(clippy::too_many_arguments)]
fn flatten_program_statements(
    statements: Vec<Stmt>,
    search_paths: &[PathBuf],
    visiting: &mut HashSet<PathBuf>,
    processed: &mut HashSet<PathBuf>,
    sources: &mut SourceMap,
    json_output: bool,
    root_file_path: &Path,
    current_at_name: &str,
    dependency_at_names: &HashSet<String>,
    at_imports: &mut Vec<(String, String)>,
    symbol_registry: &mut HashMap<String, Vec<(String, PathBuf)>>,
    module_registry: &HashMap<String, HashMap<Vec<String>, PathBuf>>,
) -> Result<Vec<Stmt>, String> {
    let mut flattened = Vec::new();

    if current_at_name.is_empty() {
        return Err("current_at_name cannot be empty in AT mode".to_string());
    }

    for stmt in statements {
        if let Stmt::Use { path } = stmt {
            let module_path_str = path.join("::");

            let is_self = path.first().map(|s| s == "@").unwrap_or(false);
            let is_dep = path
                .first()
                .map(|s| dependency_at_names.contains(s))
                .unwrap_or(false);

            if is_self || is_dep {
                if path.len() < 2 {
                    let msg = format!(
                        "AT-qualified `use` must reference an item, e.g. `use {}::name;`",
                        path.first().cloned().unwrap_or_default()
                    );
                    return Err(if json_output {
                        let json_err =
                            json_error_from_string(&root_file_path.display().to_string(), &msg);
                        serde_json::to_string(&json_err).unwrap()
                    } else {
                        format_module_error(&module_path_str, &msg)
                    });
                }

                if is_self {
                    let at_name = current_at_name.to_string();
                    let module_file_path = &path[1..];
                    let resolved_path = find_module_file(module_file_path, search_paths)
                        .ok_or_else(|| {
                            let msg = format!(
                                "Could not find module '{}' in search paths or CWD.",
                                module_file_path.join("::")
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

                    let symbols = collect_public_symbols_from_file(&resolved_path)?;
                    for sym in symbols {
                        let qualified = format!("{}${}", at_name, sym);
                        at_imports.push((sym.clone(), qualified.clone()));
                    }
                } else {
                    let dep_at_name = path[0].clone();
                    let suffix = &path[1..];

                    if let Some(module_map) = module_registry.get(&dep_at_name) {
                        if let Some(file_path) = module_map.get(suffix) {
                            let symbols = collect_public_symbols_from_file(file_path)?;
                            for sym in symbols {
                                let qualified = format!("{}${}", dep_at_name, sym);
                                at_imports.push((sym.clone(), qualified.clone()));
                            }
                            continue;
                        }
                        let symbol_name = path.last().unwrap().clone();
                        if let Some(entries) = symbol_registry.get(&dep_at_name) {
                            if !(entries.iter().any(|(sym, _)| sym == &symbol_name)) {
                                let available: Vec<_> =
                                    entries.iter().map(|(s, _)| s.as_str()).collect();
                                return Err(format!(
                                    "Symbol '{}' not exported by AT '{}' (available: {})",
                                    symbol_name,
                                    dep_at_name,
                                    available.join(", ")
                                ));
                            }
                        } else {
                            return Err(format!(
                                "AT '{}' not found in symbol registry",
                                dep_at_name
                            ));
                        }
                    } else {
                        return Err(format!("AT '{}' not found in module registry", dep_at_name));
                    }
                }
                continue;
            }

            if path.len() > 1
                && path[0]
                    .chars()
                    .next()
                    .map(|c| c.is_uppercase())
                    .unwrap_or(false)
            {
                let msg = format!(
                    "AT '{}' is not a declared dependency of '{}' (used in `use {}::..;`)",
                    path[0], current_at_name, path[0]
                );
                return Err(if json_output {
                    let json_err =
                        json_error_from_string(&root_file_path.display().to_string(), &msg);
                    serde_json::to_string(&json_err).unwrap()
                } else {
                    format_module_error(&module_path_str, &msg)
                });
            }

            let resolved_path = find_module_file(&path, search_paths).ok_or_else(|| {
                let msg = format!(
                    "Could not find module '{}' in search paths or CWD.",
                    module_path_str
                );
                if json_output {
                    let json_err =
                        json_error_from_string(&root_file_path.display().to_string(), &msg);
                    serde_json::to_string(&json_err).unwrap()
                } else {
                    format_module_error(&module_path_str, &msg)
                }
            })?;

            if visiting.contains(&resolved_path) {
                let msg = "Cyclic dependency detected! Module imports itself.".to_string();
                if json_output {
                    let json_err =
                        json_error_from_string(&root_file_path.display().to_string(), &msg);
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
                    let json_errors: Vec<JsonError> = parser
                        .parser_errs
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
                current_at_name,
                dependency_at_names,
                at_imports,
                symbol_registry,
                module_registry,
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

pub fn check_at<'a, P: AsRef<Path>>(
    ctx: &CompilerCtx<'a, P>,
    ats: &[ATInfo],
    entry: &ATEntry,
) -> Result<(), String> {
    if entry.info >= ats.len() {
        return Err(format!(
            "ATEntry.info ({}) out of bounds for {} AT(s)",
            entry.info,
            ats.len()
        ));
    }

    let order = topo_sort_ats(ats)?;
    let module_registry = build_module_registry(ats);

    let mut sources: SourceMap = HashMap::new();
    let mut merged_statements: Vec<Stmt> = Vec::new();
    let mut symbol_registry: HashMap<String, Vec<(String, PathBuf)>> = HashMap::new();

    for &idx in &order {
        let at = &ats[idx];
        let dependency_at_names: HashSet<String> =
            at.dependencies.iter().map(|d| d.name.clone()).collect();

        let file_order = sort_files_by_dependencies(&at.files, at)?;
        let search_paths = vec![at.root_dir.clone()];

        let mut all_statements = Vec::new();
        let mut at_imports = Vec::new();

        for file_path in file_order {
            let (source, tokens) = read_and_lex_file(&file_path, ctx.output_json)?;
            sources.insert(file_path.display().to_string(), source.clone());

            let mut parser = myszparser::new(tokens);
            parser.parse();

            if !parser.parser_errs.is_empty() {
                if ctx.output_json {
                    let json_errors: Vec<JsonError> = parser
                        .parser_errs
                        .iter()
                        .map(json_error_from_parser_error)
                        .collect();
                    return Err(serde_json::to_string(&json_errors).unwrap());
                } else {
                    let error_report = format_parser_errors(&parser.parser_errs, &source);
                    return Err(format!(
                        "Parser errors in file '{}':\n{}",
                        file_path.display(),
                        error_report
                    ));
                }
            }

            let mut visiting = HashSet::new();
            let mut processed = HashSet::new();
            visiting.insert(file_path.clone());

            let statements = flatten_program_statements(
                parser.ast.statements,
                &search_paths,
                &mut visiting,
                &mut processed,
                &mut sources,
                ctx.output_json,
                &file_path,
                &at.name,
                &dependency_at_names,
                &mut at_imports,
                &mut symbol_registry,
                &module_registry,
            )?;

            let filtered: Vec<Stmt> = statements
                .into_iter()
                .filter(|stmt| {
                    if let Stmt::Use { path } = stmt {
                        let is_at_use = path.first().map(|s| s == "@").unwrap_or(false)
                            || at
                                .dependencies
                                .iter()
                                .any(|d| path.first().map(|s| s == &d.name).unwrap_or(false));
                        !is_at_use
                    } else {
                        true
                    }
                })
                .collect();

            all_statements.extend(filtered);
        }

        let at_symbols = collect_symbols_from_at(at)?;
        symbol_registry.insert(at.name.clone(), at_symbols);

        let is_entry = idx == entry.info;
        let at_stmts = apply_at_aliases(all_statements, &at.name, is_entry, &at_imports);
        merged_statements.extend(at_stmts);
    }

    let program = Program {
        statements: merged_statements,
    };

    let root = &ats[entry.info].entry_file;
    let root_source = sources.get(&root.display().to_string()).map(|s| s.as_str());

    let mut analyser = Analyser::new();

    if let Err(err) = analyser.analyse(&program) {
        if ctx.output_json {
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
        AnalyserError::OverDefinitionError { location, message } => (location, message),
    };
    let file_path_str = location.file.as_ref();
    let source = sources
        .get(file_path_str)
        .map(|s| s.as_str())
        .or(root_source);
    format_error_with_location(file_path_str, location.line, location.col, message, source)
}

fn collect_exported_symbols(program: &Program) -> Vec<(String, SymbolType)> {
    let mut symbols = Vec::new();
    for stmt in &program.statements {
        match stmt {
            Stmt::Function { name, public, .. } => {
                if *public {
                    symbols.push((name.value.clone(), SymbolType::Function));
                }
            }
            Stmt::Struct { name, .. } => {
                symbols.push((name.value.clone(), SymbolType::Struct));
            }
            Stmt::Enum { name, .. } => {
                symbols.push((name.value.clone(), SymbolType::Enum));
            }
            Stmt::Constant { name, .. } => {
                symbols.push((name.value.clone(), SymbolType::Constant));
            }
            _ => {}
        }
    }
    symbols
}

fn collect_use_statements(program: &Program) -> Vec<(String, Vec<String>)> {
    let mut uses = Vec::new();
    for stmt in &program.statements {
        if let Stmt::Use { path } = stmt
            && let Some(bare_name) = path.last()
        {
            uses.push((bare_name.clone(), path.clone()));
        }
    }
    uses
}

fn topo_sort_ats(ats: &[ATInfo]) -> Result<Vec<usize>, String> {
    let name_to_idx: HashMap<&str, usize> = ats
        .iter()
        .enumerate()
        .map(|(i, at)| (at.name.as_str(), i))
        .collect();

    let mut order = Vec::new();
    let mut visited = vec![false; ats.len()];
    let mut visiting = vec![false; ats.len()];

    fn visit(
        idx: usize,
        ats: &[ATInfo],
        name_to_idx: &HashMap<&str, usize>,
        visited: &mut [bool],
        visiting: &mut [bool],
        order: &mut Vec<usize>,
    ) -> Result<(), String> {
        if visited[idx] {
            return Ok(());
        }
        if visiting[idx] {
            return Err(format!(
                "Cyclic AT dependency involving '{}'",
                ats[idx].name
            ));
        }
        visiting[idx] = true;
        for dep in &ats[idx].dependencies {
            let dep_idx = *name_to_idx.get(dep.name.as_str()).ok_or_else(|| {
                format!(
                    "AT '{}' depends on unknown AT '{}'",
                    ats[idx].name, dep.name
                )
            })?;
            visit(dep_idx, ats, name_to_idx, visited, visiting, order)?;
        }
        visiting[idx] = false;
        visited[idx] = true;
        order.push(idx);
        Ok(())
    }

    for i in 0..ats.len() {
        visit(
            i,
            ats,
            &name_to_idx,
            &mut visited,
            &mut visiting,
            &mut order,
        )?;
    }

    Ok(order)
}

fn sort_files_by_dependencies(files: &[ATFile], at: &ATInfo) -> Result<Vec<PathBuf>, String> {
    use std::collections::{HashMap, HashSet, VecDeque};

    let search_paths = vec![at.root_dir.clone()];
    let mut deps_map: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
    let mut all_files: HashSet<PathBuf> = HashSet::new();

    for file in files {
        all_files.insert(file.path.clone());
        let (_, tokens) = read_and_lex_file(&file.path, false)?;
        let mut parser = myszparser::new(tokens);
        parser.parse();

        let mut deps = Vec::new();
        for stmt in &parser.ast.statements {
            if let Stmt::Use { path } = stmt {
                let is_self = path.first().map(|s| s == "@").unwrap_or(false);
                let is_dep = at
                    .dependencies
                    .iter()
                    .any(|d| path.first().map(|s| s == &d.name).unwrap_or(false));

                if is_self {
                    let module_file_path = &path[1..];
                    if let Some(resolved) = find_module_file(module_file_path, &search_paths) {
                        deps.push(resolved);
                    }
                } else if is_dep {
                } else if let Some(resolved) = find_module_file(path, &search_paths) {
                    deps.push(resolved);
                }
            }
        }
        deps_map.insert(file.path.clone(), deps);
    }

    let mut in_degree: HashMap<PathBuf, usize> = HashMap::new();
    for file in &all_files {
        in_degree.insert(
            file.clone(),
            deps_map.get(file).map(|d| d.len()).unwrap_or(0),
        );
    }

    let mut dependents: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
    for (file, deps) in &deps_map {
        for dep in deps {
            dependents
                .entry(dep.clone())
                .or_default()
                .push(file.clone());
        }
    }

    let mut queue: VecDeque<PathBuf> = VecDeque::new();
    for (file, &deg) in &in_degree {
        if deg == 0 {
            queue.push_back(file.clone());
        }
    }

    let mut order = Vec::new();
    while let Some(file) = queue.pop_front() {
        order.push(file.clone());
        if let Some(deps_on_file) = dependents.get(&file) {
            for dependent in deps_on_file {
                if let Some(deg) = in_degree.get_mut(dependent) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(dependent.clone());
                    }
                }
            }
        }
    }

    if order.len() != all_files.len() {
        order = all_files.into_iter().collect();
    }

    Ok(order)
}

fn collect_own_top_level_names(stmts: &[Stmt], is_entry: bool) -> HashSet<String> {
    let mut names = HashSet::new();
    for stmt in stmts {
        match stmt {
            Stmt::Function { name, .. } => {
                if !(is_entry && name.value == "main") {
                    names.insert(name.value.clone());
                }
            }
            Stmt::Struct { name, .. } => {
                names.insert(name.value.clone());
            }
            Stmt::Enum { name, .. } => {
                names.insert(name.value.clone());
            }
            Stmt::Constant { name, .. } => {
                names.insert(name.value.clone());
            }
            _ => {}
        }
    }
    names
}

struct AtAliasRewriter {
    prefix: String,
    own_names: HashSet<String>,
    imports: HashMap<String, String>,
}

impl AtAliasRewriter {
    fn qualify(&self, name: &str) -> Option<String> {
        self.imports.get(name).cloned().or_else(|| {
            self.own_names
                .contains(name)
                .then(|| format!("{}{}", self.prefix, name))
        })
    }

    fn decl(&self, mut ident: Identifier) -> Identifier {
        if self.own_names.contains(&ident.value) {
            ident.value = format!("{}{}", self.prefix, ident.value);
        }
        ident
    }

    fn stmts(&self, stmts: Vec<Stmt>) -> Vec<Stmt> {
        stmts.into_iter().map(|s| self.stmt(s)).collect()
    }

    fn params(&self, params: Vec<Parameter>) -> Vec<Parameter> {
        params
            .into_iter()
            .map(|p| Parameter {
                name: p.name,
                ptype: p.ptype.map(|t| self.ty(t)),
                is_variadic: p.is_variadic,
            })
            .collect()
    }

    fn stmt(&self, stmt: Stmt) -> Stmt {
        match stmt {
            Stmt::Assignment { ident, vtype, expr } => Stmt::Assignment {
                ident,
                vtype: vtype.map(|t| self.ty(t)),
                expr: expr.map(|e| self.expr(e)),
            },
            Stmt::Constant { name, vtype, expr } => Stmt::Constant {
                name: self.decl(name),
                vtype: vtype.map(|t| self.ty(t)),
                expr: self.expr(expr),
            },
            Stmt::Reassignment { ident, expr } => Stmt::Reassignment {
                ident,
                expr: self.expr(expr),
            },
            Stmt::DerefReassignment { target, expr } => Stmt::DerefReassignment {
                target: self.expr(target),
                expr: self.expr(expr),
            },
            Stmt::Expr(e) => Stmt::Expr(self.expr(e)),
            Stmt::If {
                cond,
                then_branch,
                else_if_branches,
                else_branch,
            } => Stmt::If {
                cond: self.expr(cond),
                then_branch: self.stmts(then_branch),
                else_if_branches: else_if_branches
                    .into_iter()
                    .map(|(c, b)| (self.expr(c), self.stmts(b)))
                    .collect(),
                else_branch: else_branch.map(|b| self.stmts(b)),
            },
            Stmt::While { cond, body } => Stmt::While {
                cond: self.expr(cond),
                body: self.stmts(body),
            },
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => Stmt::For {
                init: Box::new(self.stmt(*init)),
                cond: self.expr(cond),
                step: Box::new(self.stmt(*step)),
                body: self.stmts(body),
            },
            Stmt::ForIn {
                field_ident,
                target_expr,
                body,
            } => Stmt::ForIn {
                field_ident,
                target_expr: self.expr(target_expr),
                body: self.stmts(body),
            },
            Stmt::Return { value, span } => Stmt::Return {
                value: value.map(|e| self.expr(e)),
                span,
            },
            Stmt::Use { path } => Stmt::Use { path },
            Stmt::Struct {
                name,
                generic_params,
                fields,
            } => Stmt::Struct {
                name: self.decl(name),
                generic_params,
                fields: self.params(fields),
            },
            Stmt::Enum { name, options } => Stmt::Enum {
                name: self.decl(name),
                options,
            },
            Stmt::Function {
                name,
                public,
                rttype,
                generic_params,
                params,
                body,
            } => Stmt::Function {
                name: self.decl(name),
                public,
                rttype: rttype.map(|t| self.ty(t)),
                generic_params,
                params: self.params(params),
                body: self.stmts(body),
            },
            Stmt::Extern {
                name,
                rttype,
                generic_params,
                params,
            } => Stmt::Extern {
                name,
                rttype: rttype.map(|t| self.ty(t)),
                generic_params,
                params: self.params(params),
            },
            Stmt::Break { location } => Stmt::Break { location },
        }
    }

    fn expr(&self, expr: Expr) -> Expr {
        let Expr { kind, span } = expr;
        let kind = match kind {
            ExprKind::Literal(Literal::Arr { elements }) => ExprKind::Literal(Literal::Arr {
                elements: elements.into_iter().map(|e| self.expr(e)).collect(),
            }),
            ExprKind::Literal(lit) => ExprKind::Literal(lit),
            ExprKind::Identifier(name) => ExprKind::Identifier(self.qualify(&name).unwrap_or(name)),
            ExprKind::Index { base, index } => ExprKind::Index {
                base: Box::new(self.expr(*base)),
                index: Box::new(self.expr(*index)),
            },
            ExprKind::Field { base, field } => ExprKind::Field {
                base: Box::new(self.expr(*base)),
                field,
            },
            ExprKind::StructLiteral {
                struct_name,
                generic_args,
                fields,
            } => ExprKind::StructLiteral {
                struct_name: self.qualify(&struct_name).unwrap_or(struct_name),
                generic_args: generic_args.into_iter().map(|t| self.ty(t)).collect(),
                fields: fields.into_iter().map(|(n, e)| (n, self.expr(e))).collect(),
            },
            ExprKind::Binary { left, op, right } => ExprKind::Binary {
                left: Box::new(self.expr(*left)),
                op,
                right: Box::new(self.expr(*right)),
            },
            ExprKind::Cast { left, right } => ExprKind::Cast {
                left: Box::new(self.expr(*left)),
                right: self.ty(right),
            },
            ExprKind::Unary { op, expr } => ExprKind::Unary {
                op,
                expr: Box::new(self.expr(*expr)),
            },
            ExprKind::Call {
                mut callee,
                generic_args,
                args,
            } => {
                if let Some(q) = self.qualify(&callee.value) {
                    callee.value = q;
                }
                ExprKind::Call {
                    callee,
                    generic_args: generic_args.into_iter().map(|t| self.ty(t)).collect(),
                    args: args.into_iter().map(|a| self.expr(a)).collect(),
                }
            }
            ExprKind::Sizeof { ty } => ExprKind::Sizeof { ty: self.ty(ty) },
            ExprKind::Typeof { expr } => ExprKind::Typeof {
                expr: Box::new(self.expr(*expr)),
            },
        };
        Expr { kind, span }
    }

    fn ty(&self, ty: Type) -> Type {
        match ty {
            Type::Ptr(inner) => Type::Ptr(Box::new(self.ty(*inner))),
            Type::Array { element_type, size } => Type::Array {
                element_type: Box::new(self.ty(*element_type)),
                size,
            },
            Type::Struct(name) => Type::Struct(self.qualify(&name).unwrap_or(name)),
            Type::Enum(name) => Type::Enum(self.qualify(&name).unwrap_or(name)),
            Type::GenericInstance { name, args } => Type::GenericInstance {
                name: self.qualify(&name).unwrap_or(name),
                args: args.into_iter().map(|t| self.ty(t)).collect(),
            },
            Type::VariadicPack { name, types } => Type::VariadicPack {
                name,
                types: types.into_iter().map(|t| self.ty(t)).collect(),
            },
            other => other,
        }
    }
}

fn apply_at_aliases(
    stmts: Vec<Stmt>,
    at_name: &str,
    is_entry: bool,
    imports: &[(String, String)],
) -> Vec<Stmt> {
    let own_names = collect_own_top_level_names(&stmts, is_entry);
    let mut all_names = own_names.clone();

    let import_map: HashMap<String, String> = imports.iter().cloned().collect();
    for (bare, _) in imports {
        all_names.insert(bare.clone());
    }

    let rewriter = AtAliasRewriter {
        prefix: format!("{}$", at_name),
        own_names: all_names,
        imports: import_map,
    };

    let mut result = Vec::new();
    for stmt in stmts {
        match stmt {
            Stmt::Use { path } => {
                if path.len() >= 2 {
                    let bare_name = path.last().unwrap().clone();
                    if rewriter.imports.contains_key(&bare_name) {
                        continue;
                    }
                }
                result.push(Stmt::Use { path });
            }
            _ => result.push(rewriter.stmt(stmt)),
        }
    }
    result
}

pub fn compile_at_graph<'a, P: AsRef<Path>>(
    ctx: &CompilerCtx<'a, P>,
    ats: &[ATInfo],
    entry: &ATEntry,
    output_filename: &str,
) -> Result<(), String> {
    if entry.info >= ats.len() {
        return Err(format!(
            "ATEntry.info ({}) out of bounds for {} AT(s)",
            entry.info,
            ats.len()
        ));
    }

    let at_order = topo_sort_ats(ats)?;
    let module_registry = build_module_registry(ats);

    let mut symbol_registry: HashMap<String, Vec<(String, PathBuf)>> = HashMap::new();
    let mut parsed_ats: Vec<ParsedAT> = Vec::new();
    let mut sources: SourceMap = HashMap::new();

    for &idx in &at_order {
        let at = &ats[idx];
        let dependency_at_names: HashSet<String> =
            at.dependencies.iter().map(|d| d.name.clone()).collect();

        let file_order = sort_files_by_dependencies(&at.files, at)?;

        let mut all_statements = Vec::new();
        let mut at_imports = Vec::new();
        let search_paths = vec![at.root_dir.clone()];

        for file_path in file_order {
            let (source, tokens) = read_and_lex_file(&file_path, ctx.output_json)?;
            sources.insert(file_path.display().to_string(), source.clone());

            let mut parser = myszparser::new(tokens);
            parser.parse();

            if !parser.parser_errs.is_empty() {
                if ctx.output_json {
                    let json_errors: Vec<JsonError> = parser
                        .parser_errs
                        .iter()
                        .map(json_error_from_parser_error)
                        .collect();
                    return Err(serde_json::to_string(&json_errors).unwrap());
                } else {
                    let error_report = format_parser_errors(&parser.parser_errs, &source);
                    return Err(format!(
                        "Parser errors in file '{}':\n{}",
                        file_path.display(),
                        error_report
                    ));
                }
            }

            let mut visiting = HashSet::new();
            let mut processed = HashSet::new();
            visiting.insert(file_path.clone());

            let statements = flatten_program_statements(
                parser.ast.statements,
                &search_paths,
                &mut visiting,
                &mut processed,
                &mut sources,
                ctx.output_json,
                &file_path,
                &at.name,
                &dependency_at_names,
                &mut at_imports,
                &mut symbol_registry,
                &module_registry,
            )?;

            let filtered: Vec<Stmt> = statements
                .into_iter()
                .filter(|stmt| {
                    if let Stmt::Use { path } = stmt {
                        let is_at_use = path.first().map(|s| s == "@").unwrap_or(false)
                            || at
                                .dependencies
                                .iter()
                                .any(|d| path.first().map(|s| s == &d.name).unwrap_or(false));
                        !is_at_use
                    } else {
                        true
                    }
                })
                .collect();

            all_statements.extend(filtered);
        }

        let at_symbols = collect_symbols_from_at(at)?;
        symbol_registry.insert(at.name.clone(), at_symbols);

        parsed_ats.push(ParsedAT {
            name: at.name.clone(),
            program: Program {
                statements: all_statements.clone(),
            },
            imports: at_imports
                .iter()
                .map(|(bare, qualified)| {
                    let from_at = qualified.split('$').next().unwrap_or(&at.name).to_string();
                    (
                        bare.clone(),
                        ImportInfo {
                            qualified_name: qualified.clone(),
                            from_at,
                            symbol_type: SymbolType::Function,
                        },
                    )
                })
                .collect(),
        });
    }

    let resolved_ats = resolve_imports(parsed_ats, ats)?;

    let mut merged_statements = Vec::new();
    for (topo_pos, resolved_at) in resolved_ats.iter().enumerate() {
        let original_idx = at_order[topo_pos];
        let is_entry = original_idx == entry.info;
        let at_stmts = apply_at_aliases(
            resolved_at.program.statements.clone(),
            &resolved_at.name,
            is_entry,
            &resolved_at
                .imports
                .iter()
                .map(|(k, v)| (k.clone(), v.qualified_name.clone()))
                .collect::<Vec<_>>(),
        );
        merged_statements.extend(at_stmts);
    }

    let program = Program {
        statements: merged_statements,
    };

    let root = &ats[entry.info].entry_file;
    compile_ast_program(
        &program,
        output_filename,
        &sources,
        root,
        ctx.output_json,
        &ctx.target,
    )
}

fn resolve_imports(
    mut parsed_ats: Vec<ParsedAT>,
    at_infos: &[ATInfo],
) -> Result<Vec<ParsedAT>, String> {
    let mut exported_symbols: HashMap<String, ExportInfo> = HashMap::new();

    for parsed_at in parsed_ats.iter() {
        let symbols = collect_exported_symbols(&parsed_at.program);

        for (name, sym_type) in symbols {
            let qualified = format!("{}${}", parsed_at.name, name);
            exported_symbols.insert(
                qualified,
                ExportInfo {
                    bare_name: name,
                    at_name: parsed_at.name.clone(),
                    sym_type,
                },
            );
        }
    }

    for (idx, parsed_at) in parsed_ats.iter_mut().enumerate() {
        let at_info = &at_infos[idx];
        let dependency_names: HashSet<String> = at_info
            .dependencies
            .iter()
            .map(|d| d.name.clone())
            .collect();

        let imports = collect_use_statements(&parsed_at.program);

        for (bare_name, use_path) in imports {
            let is_self = use_path.first().map(|s| s == "@").unwrap_or(false);
            let is_dep = use_path
                .first()
                .map(|s| dependency_names.contains(s))
                .unwrap_or(false);

            if is_self || is_dep {
                let at_name = if is_self {
                    parsed_at.name.clone()
                } else {
                    use_path[0].clone()
                };

                let qualified = format!("{}${}", at_name, use_path[1..].join("$"));

                if !exported_symbols.contains_key(&qualified) {
                    return Err(format!(
                        "Symbol '{}' not exported by AT '{}'",
                        bare_name, at_name
                    ));
                }

                parsed_at.imports.insert(
                    bare_name,
                    ImportInfo {
                        qualified_name: qualified,
                        from_at: at_name,
                        symbol_type: SymbolType::Function,
                    },
                );
            }
        }
    }

    Ok(parsed_ats)
}

pub fn compile_ast_program(
    program: &Program,
    output_filename: &str,
    sources: &SourceMap,
    file_path: &Path,
    json_output: bool,
    target: &CompilerTarget,
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
                AnalyserError::SemanticError { location, .. }
                | AnalyserError::OverDefinitionError { location, .. }
                | AnalyserError::TypeError { location, .. } => location,
            };

            let message = match err.clone() {
                AnalyserError::SemanticError { message, .. }
                | AnalyserError::OverDefinitionError { message, .. }
                | AnalyserError::TypeError { message, .. } => message,
            };

            let json_err = JsonError {
                file: location.file.to_string(),
                line: location.line,
                column: location.col,
                message: message.to_string(),
                severity: "error".to_string(),
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
                    is_variadic: false,
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

    for inst in irgen.code.iter().cloned() {
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
        if let Stmt::Function { name, public, .. } = stmt
            && *public
        {
            public_functions.insert(name.value.clone());
        }
    }

    match target {
        CompilerTarget::Cranelift => compile_with_cranelift(
            irgen,
            analyser.functions.clone(),
            tac_instructions,
            public_functions,
            file_path,
            output_filename,
        ),

        CompilerTarget::Llvm => compile_with_llvm(
            irgen,
            analyser.functions.clone(),
            tac_instructions,
            public_functions,
            file_path,
            output_filename,
        ),
    }
}

fn compile_with_cranelift(
    irgen: IRGen,
    functions: HashMap<String, FunctionSignature>,
    tac_instructions: Vec<Instruction>,
    public_functions: HashSet<String>,
    file_path: &Path,
    output_filename: &str,
) -> Result<(), String> {
    let mut unique_function_names = HashSet::new();

    for inst in &tac_instructions {
        if let Instruction::FunctionLabel(name) = inst {
            unique_function_names.insert(name.clone());
        }
    }

    let mut backend = clback::CraneliftBackend::new(irgen.struct_defs, functions);

    backend.register_defined_functions(unique_function_names.iter().cloned());

    backend.scan_externs(&tac_instructions);

    let instruction_refs: Vec<&Instruction> = tac_instructions.iter().collect();

    backend.pre_declare_strings(&instruction_refs);

    for func_name in unique_function_names {
        let is_public = public_functions.contains(&func_name);

        let func_instructions: Vec<&Instruction> = tac_instructions
            .iter()
            .skip_while(|inst| {
                !matches!(
                    inst,
                    Instruction::FunctionLabel(name)
                        if name == &func_name
                )
            })
            .skip(1)
            .take_while(|inst| !matches!(inst, Instruction::FunctionLabel(_)))
            .collect();

        if !func_instructions.is_empty() {
            let mut ctx = clContext::new();
            let mut func_ctx = clFunctionBuilderContext::new();

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

    write_output_file(file_path, output_filename, &emit_result)
}

fn is_generic_type(ty: &Type) -> bool {
    matches!(
        ty,
        Type::GenericParam(_)
            | Type::GenericInstance { .. }
            | Type::VariadicPack { .. }
            | Type::Any
    )
}

#[allow(unused)]
fn compile_with_llvm(
    irgen: IRGen,
    functions: HashMap<String, FunctionSignature>,
    tac_instructions: Vec<Instruction>,
    public_functions: HashSet<String>,
    file_path: &Path,
    output_filename: &str,
) -> Result<(), String> {
    let concrete_functions: HashMap<String, FunctionSignature> = functions
        .into_iter()
        .filter(|(_, sig)| {
            !sig.param_types.iter().any(is_generic_type) && !is_generic_type(&sig.return_type)
        })
        .collect();

    let context = inkContext::create();

    Target::initialize_native(&InitializationConfig::default()).map_err(|e| e.to_string())?;

    let modname = file_path.file_name();

    if modname.is_none() {
        return Err("filepath doesn't containe a file".to_string());
    }

    let mut backend = LlvmBackend::new(
        &context,
        &modname.unwrap().to_string_lossy(),
        irgen.var_types,
        irgen.struct_defs,
        concrete_functions,
    );

    backend.compile(&tac_instructions)?;

    backend.verify()?;

    let target_triple = TargetMachine::get_default_triple();

    let target = Target::from_triple(&target_triple).map_err(|e| e.to_string())?;

    let target_machine = target
        .create_target_machine(
            &target_triple,
            "generic",
            "",
            OptimizationLevel::None,
            RelocMode::PIC,
            CodeModel::Default,
        )
        .ok_or_else(|| "failed to create LLVM target machine".to_string())?;

    target_machine
        .write_to_file(
            backend.module(),
            FileType::Object,
            Path::new(&output_filename),
        )
        .map_err(|e| e.to_string())?;

    Ok(())
}

fn write_output_file(file_path: &Path, output_filename: &str, bytes: &[u8]) -> Result<(), String> {
    let mut file = File::create(output_filename).map_err(|e| {
        format_simple_error(
            file_path,
            &format!("Failed to create output file '{}': {}", output_filename, e),
        )
    })?;

    file.write_all(bytes).map_err(|e| {
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
