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

    Eq, // ==
    NEq, // !=
    Gt, // >
    GtE, // >=
    Lt, // <
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

    Arg { value: Value },
    Call { dest: Option<String>, name: String, argc: usize },

    Store {ptr: Value, source: Value},

    Load { dst: String, ptr: Value, ty: Type },

    Extern { fnname: String }
}