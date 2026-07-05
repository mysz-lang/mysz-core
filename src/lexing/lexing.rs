use crate::utils::location::Location;

#[derive(Debug, Clone, PartialEq)]
pub enum TokenType {
    // boolish
    Equals,
    NotEquals,
    LessThan,
    GreaterThan,
    LessThanEquals,
    GreaterThanEquals,



    // generic signs
    Assign,
    LParen,
    RParen,
    LBrace,
    RBrace,
    SemiColon,
    Colon,
    Comma,
    
    // maths signs
    Add, // +
    Minus, // -
    Divide, // /
    Multiply, // *
    Modulo, // %
    Not,    // !
    
    // boolean values
    True,
    False,

    // keywords
    VarKeyword,
    IfKeyword,
    ElseKeyword,
    WhileKeyword,
    FnKeyword,
    ReturnKeyword,
    ExternKeyword,

    // identifier
    Identifier,
    
    // literals
    IntLiteral,
    StringLiteral,

    // when lexing, parsing, etc fails.
    Niltoken
}

#[derive(Clone, Debug)]
pub struct Token {
    pub ttype: TokenType,
    pub location: Location,
    pub value: String
}
impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Token {{ Type: {:?}, Location: ({}, {}), Value: {} }}",
            self.ttype,
            self.location.line,
            self.location.col,
            self.value
        )
    }
}