use std::collections::HashMap;

use crate::{parse::parsing::Type, utils::location::Location};

#[derive(Debug)]
pub struct Symbol {
    pub name: String,
    pub ty: Type,
}

#[derive(Debug)]
pub struct Scope {
    pub symbols: HashMap<String, Symbol>,
    pub parent: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct StructSignature {
    pub generic_params: Vec<String>,
    pub fields: HashMap<String, Type>,
    pub location: Location,
}

#[derive(Clone, Debug)]
pub struct FunctionSignature {
    pub generic_params: Vec<String>,
    pub param_types: Vec<Type>,
    pub return_type: Type,
    pub location: Location,
}
