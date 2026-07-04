use crate::lexing::tokens::{Token, TokenType};
use crate::utils::location::Location;

pub struct Lexer {
    pub source: String,
    pub token_idx: usize,
    pub line: usize,
    pub col: usize,
    pub tokens: Vec<Token>,
}
impl Lexer{
    pub fn new(source: String) -> Self {
        Self { source, token_idx: 0 as usize, tokens: Vec::new(), line: 0 as usize, col: 0 as usize }
    }

    pub fn current_location(&self) -> Location {
        Location::new(self.line, self.col)
    }

    pub fn advance(&mut self) {
        if let Some(ch) = self.get_char() {
            if ch == '\n' {
                self.line += 1;
                self.col = 0;
            } else {
                self.col += 1;
            }
        }

        self.token_idx += 1;
    }

    pub fn peek(&self, offset: i32) -> Option<char> {
        let true_offset = self.token_idx + offset as usize;

        self.source
            .get(true_offset..)
            .and_then(|s| s.chars().next())
    }

    pub fn get_char(&self) -> Option<char> {
        self.peek(0)
    }

    pub fn add_token(&mut self, token: Token) {
        self.tokens.push(token);
    }

    // used for single character tokens, just supply the TokenType and it will be added to tokens (I don't want to write a bunch of token definitions for single character tokens, lazy ahh bum)
    pub fn single_char(&mut self, ttype: TokenType) {
        let ch = self.get_char().expect("Unexpected EOF while lexing single char token");

        let t = Token {
            ttype,
            location: self.current_location(),
            value: ch.to_string(),
        };

        self.add_token(t);
        self.advance();
    }

    pub fn lex(&mut self) {
        while let Some(ch) = self.get_char() {

            if char::is_numeric(ch) {
                let t = self.lex_numeric();
                self.add_token(t);

            } else if char::is_alphabetic(ch) {
                let t = self.lex_identifier_and_keyword();
                self.add_token(t);

            } else {

                match ch {
                    '=' => {let t = self.lex_assign(); self.add_token(t);},
                    
                    '"' => {let t = self.lex_string(); self.add_token(t);},

                    ';' => {self.single_char(TokenType::SemiColon);}
                    '(' => {self.single_char(TokenType::LParen);}
                    ')' => {self.single_char(TokenType::RParen);}
                    '{' => {self.single_char(TokenType::LBrace);}
                    '}' => {self.single_char(TokenType::RBrace);},

                    '+' => {self.single_char(TokenType::Add);}
                    '-' => {self.single_char(TokenType::Minus);}
                    '*' => {self.single_char(TokenType::Multiply);}
                    '/' => {self.single_char(TokenType::Divide);}
                    '%' => {self.single_char(TokenType::Modulo);}

                    _ => self.advance(),
                }

            }
        }
    }

    pub fn lex_assign(&mut self) -> Token {
        let loc = self.current_location();
        let current = self.get_char().expect("Unexpected EOF at '='");

        if self.peek(1) == Some('=') {
            let next = self.peek(1).unwrap();

            self.advance(); // first '='
            self.advance(); // second '='

            return Token {
                ttype: TokenType::Equals,
                location: loc,
                value: format!("{}{}", current, next),
            };
        }

        self.advance(); // '='

        Token {
            ttype: TokenType::Assign,
            location: loc,
            value: current.to_string(),
        }
    }

    pub fn lex_string(&mut self) -> Token {
        let loc = self.current_location();

        let mut string: Vec<char> = Vec::new();

        self.advance(); // skip initial '"'

        while let Some(ch) = self.get_char() {
            if ch != '"' {
                string.push(ch);
                self.advance();
            } else {
                self.advance(); // skip closing '"'
                break;
            }
        }

        let value: String = string.into_iter().collect();

        Token {
            ttype: TokenType::StringLiteral,
            location: loc,
            value
        }
    }

    pub fn lex_numeric(&mut self) -> Token {
        let loc = self.current_location();

        let mut numstring: Vec<char> = Vec::new();

        while let Some(ch) = self.get_char() {
            if ch.is_numeric() {
                numstring.push(ch);
                self.advance();
            } else {
                break;
            }
        }

        let value: String = numstring.into_iter().collect();

        Token {
            // adjust these fields to your actual struct
            ttype: TokenType::IntLiteral,
            location: loc,
            value,
        }
    }

    pub fn lex_identifier_and_keyword(&mut self) -> Token {
        let loc = self.current_location();
        let mut buf: Vec<char> = Vec::new();

        while let Some(ch) = self.get_char() {
            if ch.is_alphanumeric() || ch == '_' {
                buf.push(ch);
                self.advance();
            } else {
                break;
            }
        }

        let value: String = buf.into_iter().collect();

        match value.as_str() {
            "var" => {return Token {
                ttype: TokenType::VarKeyword,
                location: loc,
                value
            }},
            "if" => {return Token {
                ttype: TokenType::Ifkeyword,
                location: loc,
                value
            }}

            _ => {}
        }

        Token {
            ttype: TokenType::Identifier,
            location: loc,
            value
        }
    }
}