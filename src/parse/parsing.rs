use crate::utils::location::Location;
use crate::utils::typesafe::Type;

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Int(i64),
    String(String),
    Char(char),
    Bool(bool),
    Arr { elements: Vec<Expr> },
    Double(f64),
    Float(f32),
    Nil,
}
impl Literal {
    pub fn to_i64(&self) -> i64 {
        match self {
            Literal::Int(n) => *n,
            Literal::Double(n) => *n as i64,
            Literal::Float(n) => *n as i64,
            _ => panic!("Expected integer literal"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,

    Eq,
    NEq,
    Gt,
    GtE,
    Lt,
    LtE,
    And,
    Or,
}
#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    Positive,
    Negative,
    AddressOf,
    Deref,
    Not,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Identifier {
    pub value: String,
    pub location: Location,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Literal(Literal),
    Identifier(String),

    // array indexing
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },

    // struct literal
    Field {
        base: Box<Expr>,
        field: String,
    },
    StructLiteral {
        struct_name: String,
        generic_args: Vec<Expr>,
        fields: Vec<(String, Expr)>,
    },

    // basic maths
    Binary {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
    },

    Cast {
        left: Box<Expr>,
        right: Type,
    },

    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },

    Call {
        callee: Identifier,
        generic_args: Vec<Expr>,
        args: Vec<Expr>,
    },
    Sizeof {
        ty: Type,
    },
    Typeof {
        expr: Box<Expr>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Location,
}

#[derive(Debug, Clone)]
pub struct Parameter {
    pub name: Identifier,
    pub ptype: Option<Type>,
    pub is_variadic: bool,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Assignment {
        ident: Identifier,
        vtype: Option<Type>,
        expr: Option<Expr>,
    },
    Constant {
        name: Identifier,
        vtype: Option<Type>,
        expr: Expr,
    },
    Reassignment {
        ident: Identifier,
        expr: Expr,
    },
    DerefReassignment {
        target: Expr,
        expr: Expr,
    },
    Expr(Expr),
    If {
        cond: Expr,
        then_branch: Vec<Stmt>,
        else_if_branches: Vec<(Expr, Vec<Stmt>)>,
        else_branch: Option<Vec<Stmt>>,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
    For {
        init: Box<Stmt>,
        cond: Expr,
        step: Box<Stmt>,
        body: Vec<Stmt>,
    },
    ForIn {
        field_ident: Identifier,
        target_expr: Expr,
        body: Vec<Stmt>,
    },
    Return {
        value: Option<Expr>,
        span: Location,
    },
    Use {
        path: Vec<String>,
    },
    Struct {
        name: Identifier,
        generic_params: Vec<String>,
        fields: Vec<Parameter>,
    },
    Enum {
        name: Identifier,
        options: Vec<Identifier>,
    },
    Function {
        name: Identifier,
        public: bool,
        rttype: Option<Type>,
        generic_params: Vec<String>,
        params: Vec<Parameter>,
        body: Vec<Stmt>,
    },
    Extern {
        name: Identifier,
        rttype: Option<Type>,
        generic_params: Vec<String>,
        params: Vec<Parameter>,
    },
    Break {
        location: Location,
    },
}

#[derive(Debug)]
pub struct Program {
    pub statements: Vec<Stmt>,
}

#[derive(Debug)]
pub enum ParserErrorType {
    MalformedStatementError,
    UnexpectedTokenTypeError,
    UnimplementedError,
}

#[derive(Debug)]
pub struct ParserError {
    pub etype: ParserErrorType,
    pub message: String,
    pub location: Location,
}
impl std::fmt::Display for ParserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "! Parser Error :{}: {:?}: {}",
            self.location, self.etype, self.message
        )
    }
}
