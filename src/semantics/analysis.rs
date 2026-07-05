use std::collections::HashMap;

use crate::{parse::parsing::Identifier, utils::location::Location};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type { Int, Str, Bool, Void }

impl Type {
    pub fn from_ident(ident: &Identifier) -> Result<Self, String> {
        match ident.value.as_str() {
            "int" => Ok(Type::Int),
            "str" => Ok(Type::Str),
            "bool" => Ok(Type::Bool),
            "void" => Ok(Type::Void),
            _ => Err(format!(
                "Semantic Error [{}]: Unknown type annotation '{}'", 
                ident.location, ident.value
            )),
        }
    }
}

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