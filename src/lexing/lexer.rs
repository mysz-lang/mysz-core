use crate::lexing::lexing::TokenType::{GreaterThanEquals, LessThanEquals};
use crate::lexing::lexing::{Token, TokenType};
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

    fn current_location(&self) -> Location {
        Location::new(self.line, self.col)
    }

    fn advance(&mut self) {
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

    fn peek(&self, offset: i32) -> Option<char> {
        let true_offset = self.token_idx + offset as usize;

        self.source
            .get(true_offset..)
            .and_then(|s| s.chars().next())
    }

    pub fn get_char(&self) -> Option<char> {
        self.peek(0)
    }

    fn add_token(&mut self, token: Token) {
        self.tokens.push(token);
    }

    // used for single character tokens, just supply the TokenType and it will be added to tokens (I don't want to write a bunch of token definitions for single character tokens, lazy ahh bum)
    fn single_char(&mut self, ttype: TokenType) {
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
                    '=' => {let t = self.lex_assign(); self.add_token(t);}
                    
                    '"' => {let t = self.lex_string(); self.add_token(t);}

                    '!' => {let t = self.lex_not(); self.add_token(t);}

                    '>' => {let t = self.lex_gt(); self.add_token(t);}
                    '<' => {let t = self.lex_lt(); self.add_token(t);}

                    '&' => {self.single_char(TokenType::Ampersand);}
                    '^' => {self.single_char(TokenType::Star);}

                    ';' => {self.single_char(TokenType::SemiColon);}
                    ':' => {self.single_char(TokenType::Colon);}
                    '(' => {self.single_char(TokenType::LParen);}
                    ')' => {self.single_char(TokenType::RParen);}
                    '{' => {self.single_char(TokenType::LBrace);}
                    '}' => {self.single_char(TokenType::RBrace);}
                    ',' => {self.single_char(TokenType::Comma);}
                    '[' => {self.single_char(TokenType::LBracket);}
                    ']' => {self.single_char(TokenType::RBracket);}
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

    fn lex_gt(&mut self) -> Token {
        let loc = self.current_location();
        let current = self.get_char().expect("Unexpected EOF at '>'");

        if self.peek(1) == Some('=') {
            let next = self.peek(1).unwrap();

            self.advance(); self.advance();

            return Token {
                ttype: GreaterThanEquals,
                location: loc,
                value: format!("{}{}", current, next)
            }
        }

        self.advance();

        Token {
            ttype: TokenType::GreaterThan,
            location: loc,
            value: current.to_string()
        }
    }

    fn lex_lt(&mut self) -> Token {
        let loc = self.current_location();
        let current = self.get_char().expect("Unexpected EOF at '<'");

        if self.peek(1) == Some('=') {
            let next = self.peek(1).unwrap();

            self.advance(); self.advance();

            return Token {
                ttype: LessThanEquals,
                location: loc,
                value: format!("{}{}", current, next)
            }
        }

        self.advance();

        Token {
            ttype: TokenType::LessThan,
            location: loc,
            value: current.to_string()
        }
    }

    fn lex_not(&mut self) -> Token {
        let loc = self.current_location();
        let current = self.get_char().expect("Unexpected EOF at '!'");

        if self.peek(1) == Some('=') {
            let next = self.peek(1).unwrap();

            self.advance(); // !
            self.advance(); // =

            return Token {
                ttype: TokenType::NotEquals,
                location: loc,
                value: format!("{}{}", current, next)
            }
        }

        self.advance();

        Token {
            ttype: TokenType::Not,
            location: loc,
            value: current.to_string()
        }
    }

    fn lex_assign(&mut self) -> Token {
        let loc = self.current_location();
        let current = self.get_char().expect("Unexpected EOF at '='");

        if self.peek(1) == Some('=') {
            let next = self.peek(1).unwrap();

            self.advance(); // =
            self.advance(); // =

            return Token {
                ttype: TokenType::Equals,
                location: loc,
                value: format!("{}{}", current, next),
            };
        }

        self.advance(); // =

        Token {
            ttype: TokenType::Assign,
            location: loc,
            value: current.to_string(),
        }
    }

    fn lex_string(&mut self) -> Token {
        let loc = self.current_location();

        let mut string: Vec<char> = Vec::new();

        self.advance(); // "

        while let Some(ch) = self.get_char() {
            if ch != '"' {
                string.push(ch);
                self.advance();
            } else {
                self.advance(); // "
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

    fn lex_numeric(&mut self) -> Token {
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
            ttype: TokenType::IntLiteral,
            location: loc,
            value,
        }
    }

    fn lex_identifier_and_keyword(&mut self) -> Token {
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
                ttype: TokenType::IfKeyword,
                location: loc,
                value
            }},
            "else" => {return Token {
                ttype: TokenType::ElseKeyword,
                location: loc,
                value
            }}
            "while" => {return Token {
                ttype: TokenType::WhileKeyword,
                location: loc,
                value
            }},
            "fn" => {return Token {
                ttype: TokenType::FnKeyword,
                location: loc,
                value
            }},
            "return" => {return Token {
                ttype: TokenType::ReturnKeyword,
                location: loc,
                value
            }},
            "extern" => {return Token {
                ttype: TokenType::ExternKeyword,
                location: loc,
                value
            }}
            "true" => {
                return Token {
                ttype: TokenType::True,
                location: loc,
                value
            }},
            "false" => {
                return Token {
                    ttype: TokenType::False,
                    location: loc,
                    value
            }},

            _ => {}
        }

        Token {
            ttype: TokenType::Identifier,
            location: loc,
            value
        }
    }
}