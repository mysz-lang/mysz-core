#[derive(Debug)]
pub enum Literal {
    Int(String),
    String(String),
}

#[derive(Debug)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}
#[derive(Debug)]
pub enum UnaryOp {
    Positive,
    Negative
}

pub type Identifier = String;
#[derive(Debug)]
pub enum Expr {
    // building blocks
    Literal(Literal),
    Identifier(Identifier),

    // basic maths
    Binary {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>
    },

    Unary {
        op: UnaryOp,
        expr: Box<Expr>
    }
}

#[derive(Debug)]
pub enum Stmt {
    Assignment{
        ident: Identifier, 
        expr: Expr
    },
    Expr(Expr),
    If{
        cond: Expr, 
        then_branch: Vec<Stmt>
    }
}

pub struct Program {
    pub statements: Vec<Stmt>,
}