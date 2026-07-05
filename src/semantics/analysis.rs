use std::collections::HashMap;

use crate::{parse::parsing::{Type}, utils::location::Location};

pub struct Symbol {
    pub data_type: Type,
}

pub struct Scope {
    pub symbols: HashMap<String, Symbol>,
    pub parent: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct FunctionSignature {
    pub param_types: Vec<Type>,
    pub return_type: Type,
    pub location: Location,
}