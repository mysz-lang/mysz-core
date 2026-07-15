use std::collections::HashMap;

use crate::parse::parsing::Type;

#[derive(Debug, Clone, PartialEq)]
pub enum IrOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod, // %

    Neg, // unary minus
    Pos, // unary plus
    Ref, // unary &
    Not, // unary !

    Eq,  // ==
    NEq, // !=
    Gt,  // >
    GtE, // >=
    Lt,  // <
    LtE, // <=
}

#[derive(Debug, Clone)]
pub enum Value {
    Const(i64),
    Temp(String),
    Var(String),
    Void,

    Str(String),
    Char(char),
    Bool(bool),
}

#[derive(Debug, Clone)]
pub enum Instruction {
    Assign {
        dst: String,
        src: Value,
    },

    Binary {
        dst: String,
        op: IrOp,
        lhs: Value,
        rhs: Value,
    },

    Unary {
        dst: String,
        op: IrOp,
        value: Value,
    },

    Label(String),
    FunctionLabel(String),

    Jump(String),

    JumpIfFalse {
        cond: Value,
        target: String,
    },

    Param {
        p: String,
    },

    Return {
        value: Value,
    },

    Arg {
        value: Value,
    },
    Call {
        dest: Option<String>,
        name: String,
        argc: usize,
    },

    Store {
        ptr: Value,
        source: Value,
    },

    Load {
        dst: String,
        ptr: Value,
        ty: Type,
    },

    Extern {
        fnname: String,
    },
}

#[derive(Debug, Clone)]
pub struct ScopedMap {
    scopes: Vec<HashMap<String, Type>>,
}

impl ScopedMap {
    pub fn new(initial: HashMap<String, Type>) -> Self {
        Self {
            scopes: vec![initial],
        }
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub fn pop_scope(&mut self) {
        if self.scopes.len() > 1 {
            let popped = self.scopes.pop().unwrap();
            if let Some(parent) = self.scopes.last_mut() {
                for (key, value) in popped {
                    parent.entry(key).or_insert(value);
                }
            }
        }
    }

    pub fn insert(&mut self, key: String, value: Type) {
        if let Some(current_scope) = self.scopes.last_mut() {
            current_scope.insert(key, value);
        }
    }

    pub fn get(&self, key: &str) -> Option<&Type> {
        for scope in self.scopes.iter().rev() {
            if let Some(ty) = scope.get(key) {
                return Some(ty);
            }
        }
        None
    }
}