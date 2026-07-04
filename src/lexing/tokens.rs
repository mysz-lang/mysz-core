use crate::utils::location::Location;

#[derive(Debug, Clone, PartialEq)]
pub enum TokenType {
    // generic signs
    Equals,
    Assign,
    LParen,
    RParen,
    LBrace,
    RBrace,
    SemiColon,
    
    // maths signs
    Add, // +
    Minus, // -
    Divide, // /
    Multiply, // *
    Modulo, // %
    


    // keywords
    VarKeyword,
    Ifkeyword,

    // identifier
    Identifier,
    
    // literals
    IntLiteral,
    StringLiteral,
}

#[derive(Clone)]
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