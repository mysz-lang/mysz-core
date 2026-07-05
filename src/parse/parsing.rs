use crate::utils::location::Location;



#[derive(Debug, Clone)]
pub enum Literal {
    Int(i64),
    String(String),
    Bool(bool)
}
impl Literal {
    pub fn to_i64(&self) -> i64 {
        match self {
            Literal::Int(n) => *n,
            _ => panic!("Expected integer literal"),
        }
    }
}

#[derive(Debug, Clone)]
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
}
#[derive(Debug, Clone)]
pub enum UnaryOp {
    Positive,
    Negative
}

#[derive(Debug, Clone)]
pub struct Identifier {
    pub value: String,
    pub location: Location
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    // building blocks
    Literal(Literal),
    Identifier(String),

    // basic maths
    Binary {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>
    },

    Unary {
        op: UnaryOp,
        expr: Box<Expr>
    },

    Call {
        callee: Identifier,
        args: Vec<Expr>
    }
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Location,
}

#[derive(Debug, Clone)]
pub struct Parameter {
    pub name: Identifier,
    pub ptype: Option<Identifier>
}


#[derive(Debug)]
pub enum Stmt {
    Assignment{
        ident: Identifier,
        vtype: Option<Identifier>,
        expr: Expr
    },
    Reassignment{
        ident: Identifier,
        expr: Expr
    },
    Expr(Expr),
    If{
        cond: Expr, 
        then_branch: Vec<Stmt>
    },
    While{
        cond: Expr,
        body: Vec<Stmt>
    },
    Function{
        name: Identifier,
        rttype: Option<Identifier>,
        params: Vec<Parameter>,
        body: Vec<Stmt>
    },
    Return{
        value: Option<Expr>,
        span: Location
    },
    Extern{
        name: Identifier,
        rttype: Option<Identifier>,
        params: Vec<Parameter>,
    }
}

#[derive(Debug)]
pub struct Program {
    pub statements: Vec<Stmt>,
}


#[derive(Debug)]
pub enum ParserErrorType {
    MalformedStatementError,
    UnexpectedTokenTypeError,
    UnimplementedError
}

#[derive(Debug)]
pub struct ParserError {
    pub etype: ParserErrorType,
    pub message: String,
    pub location: Location
}
impl std::fmt::Display for ParserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "! Parser Error :{}: {:?}: {}",
            self.location,
            self.etype,
            self.message
        )
    }
}