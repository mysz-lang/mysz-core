// In utils/ats.rs
use indexmap::IndexMap;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use crate::{
    ir::irgen::StructLayout,
    parse::parsing::{Expr, Parameter, Program, Stmt, Type},
};

#[derive(Debug, Clone)]
pub struct ATInfo {
    pub name: String,
    pub version: Option<String>,
    pub root_dir: PathBuf,
    pub entry_file: PathBuf,
    pub files: Vec<ATFile>,
    pub dependencies: Vec<ATDependency>,

    // this will be filled by the compiler, wrappers shouldn't care about this
    pub struct_defs: HashMap<String, StructLayout>,
    pub struct_blueprints: HashMap<String, (Vec<String>, Vec<Parameter>)>,
    pub enum_defs: HashMap<String, IndexMap<String, i64>>,
    pub fn_blueprints: HashMap<String, Stmt>,
    pub analyser_constants: HashMap<String, (Type, Expr)>,
    pub exported: HashSet<String>,
}

pub struct ParsedAT {
    pub name: String,
    pub program: Program,
    pub imports: HashMap<String, ImportInfo>,
}

pub struct ImportInfo {
    pub qualified_name: String,
    pub from_at: String,
    pub symbol_type: SymbolType,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SymbolType {
    Function,
    Struct,
    Enum,
    Constant,
}

pub struct ExportInfo {
    pub bare_name: String,
    pub at_name: String,
    pub sym_type: SymbolType,
}

#[derive(Debug, Clone)]
pub struct ATFile {
    pub path: PathBuf,
    pub module_path: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ATDependency {
    pub name: String,
    pub version_req: Option<String>,
}

pub struct ATEntry {
    pub info: usize,
    pub is_current: bool,
}
