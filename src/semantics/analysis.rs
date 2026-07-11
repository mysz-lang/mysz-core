use std::collections::HashMap;

use crate::{parse::parsing::Type, utils::location::Location};

pub struct Symbol {
    pub data_type: Type,
}

pub struct Scope {
    pub symbols: HashMap<String, Symbol>,
    pub parent: Option<usize>,
}

#[derive(Clone)]
pub struct StructSignature {
    pub generic_params: Vec<String>, // Added to keep track of templates
    pub fields: HashMap<String, Type>,
    pub location: Location,
}

#[derive(Clone)]
pub struct FunctionSignature {
    pub generic_params: Vec<String>, // Added to keep track of templates
    pub param_types: Vec<Type>,
    pub return_type: Type,
    pub location: Location,
}
