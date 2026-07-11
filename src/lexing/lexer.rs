use crate::lexing::lexing::{Token, TokenType};
use crate::utils::location::Location;

#[derive(Debug, Clone)]
pub enum LexError {
    UnexpectedEof { context: &'static str, location: Location },
    UnknownEscapeSequence { ch: char, location: Location },
    UnterminatedCharLiteral { location: Location },
    UnterminatedComment { location: Location },
}

impl std::fmt::Display for LexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LexError::UnexpectedEof { context, location } => {
                write!(f, "Unexpected EOF while lexing {} at {:?}", context, location)
            }
            LexError::UnknownEscapeSequence { ch, location } => {
                write!(f, "Unknown escape sequence '\\{}' at {:?}", ch, location)
            }
            LexError::UnterminatedCharLiteral { location } => {
                write!(f, "Unterminated character literal at {:?}", location)
            }
            LexError::UnterminatedComment { location } => {
                write!(f, "Unterminated multi-line comment at {:?}", location)
            }
        }
    }
}
impl std::error::Error for LexError {}

pub struct Lexer {
    pub source: String,
    pub token_idx: usize,
    pub line: usize,
    pub col: usize,
    pub tokens: Vec<Token>,
}

impl Lexer {
    pub fn new(source: String) -> Self {
        Self {
            source,
            token_idx: 0,
            tokens: Vec::new(),
            line: 0,
            col: 0,
        }
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
        self.source.get(true_offset..).and_then(|s| s.chars().next())
    }

    pub fn get_char(&self) -> Option<char> {
        self.peek(0)
    }

    fn add_token(&mut self, token: Token) {
        self.tokens.push(token);
    }

    fn single_char(&mut self, ttype: TokenType) -> Result<(), LexError> {
        let ch = self.get_char().ok_or(LexError::UnexpectedEof {
            context: "single char token",
            location: self.current_location(),
        })?;

        let t = Token {
            ttype,
            location: self.current_location(),
            value: ch.to_string(),
        };

        self.add_token(t);
        self.advance();
        Ok(())
    }

    pub fn lex(&mut self) -> Result<(), LexError> {
        while let Some(ch) = self.get_char() {
            if char::is_numeric(ch) {
                let t = self.lex_numeric();
                self.add_token(t);
            } else if char::is_alphabetic(ch) {
                let t = self.lex_identifier_and_keyword();
                self.add_token(t);
            } else {
                match ch {
                    '=' => {
                        let t = self.lex_assign()?;
                        self.add_token(t);
                    }
                    '"' => {
                        let t = self.lex_string();
                        self.add_token(t);
                    }
                    '\'' => {
                        let t = self.lex_char()?;
                        self.add_token(t);
                    }
                    '!' => {
                        let t = self.lex_not()?;
                        self.add_token(t);
                    }
                    '>' => {
                        let t = self.lex_gt()?;
                        self.add_token(t);
                    }
                    '<' => {
                        let t = self.lex_lt()?;
                        self.add_token(t);
                    }
                    '.' => self.single_char(TokenType::Period)?,
                    '&' => self.single_char(TokenType::Ampersand)?,
                    '^' => self.single_char(TokenType::Star)?,
                    ':' => {
                        let t = self.lex_colon()?;
                        self.add_token(t);
                    }
                    ';' => self.single_char(TokenType::SemiColon)?,
                    '(' => self.single_char(TokenType::LParen)?,
                    ')' => self.single_char(TokenType::RParen)?,
                    '{' => self.single_char(TokenType::LBrace)?,
                    '}' => self.single_char(TokenType::RBrace)?,
                    ',' => self.single_char(TokenType::Comma)?,
                    '[' => self.single_char(TokenType::LBracket)?,
                    ']' => self.single_char(TokenType::RBracket)?,
                    '+' => self.single_char(TokenType::Add)?,
                    '-' => self.single_char(TokenType::Minus)?,
                    '*' => self.single_char(TokenType::Multiply)?,
                    '/' => {
                        if let Some(t) = self.lex_slash()? {
                            self.add_token(t);
                        }
                    }
                    '%' => self.single_char(TokenType::Modulo)?,
                    _ => self.advance(),
                }
            }
        }
        Ok(())
    }

    fn lex_slash(&mut self) -> Result<Option<Token>, LexError> {
        let loc = self.current_location();
        let current = self.get_char().ok_or(LexError::UnexpectedEof {
            context: "'/'",
            location: loc.clone(),
        })?;

        if self.peek(1) == Some('/') {
            if self.peek(2) == Some('\'') {
                // Multi-line comment: //' ... '//
                self.advance(); // /
                self.advance(); // /
                self.advance(); // '

                loop {
                    match self.get_char() {
                        None => {
                            return Err(LexError::UnterminatedComment {
                                location: self.current_location(),
                            });
                        }
                        Some('\'') if self.peek(1) == Some('/') && self.peek(2) == Some('/') => {
                            self.advance();
                            self.advance();
                            self.advance();
                            break;
                        }
                        Some(_) => self.advance(),
                    }
                }
            } else {
                self.advance();
                self.advance();
                while let Some(ch) = self.get_char() {
                    if ch == '\n' {
                        break;
                    }
                    self.advance();
                }
            }
            return Ok(None);
        }

        self.advance();

        Ok(Some(Token {
            ttype: TokenType::Divide,
            location: loc,
            value: current.to_string(),
        }))
    }

    fn lex_colon(&mut self) -> Result<Token, LexError> {
        let loc = self.current_location();
        let current = self.get_char().ok_or(LexError::UnexpectedEof {
            context: "':'",
            location: loc.clone(),
        })?;

        if self.peek(1) == Some(':') {
            let next = self.peek(1).unwrap();
            self.advance();
            self.advance();
            return Ok(Token {
                ttype: TokenType::DoubleColon,
                location: loc,
                value: format!("{}{}", current, next),
            });
        }

        self.advance();
        Ok(Token {
            ttype: TokenType::Colon,
            location: loc,
            value: current.to_string(),
        })
    }

    fn lex_gt(&mut self) -> Result<Token, LexError> {
        let loc = self.current_location();
        let current = self.get_char().ok_or(LexError::UnexpectedEof {
            context: "'>'",
            location: loc.clone(),
        })?;

        if self.peek(1) == Some('=') {
            let next = self.peek(1).unwrap();
            self.advance();
            self.advance();
            return Ok(Token {
                ttype: TokenType::GreaterThanEquals,
                location: loc,
                value: format!("{}{}", current, next),
            });
        }

        self.advance();
        Ok(Token {
            ttype: TokenType::GreaterThan,
            location: loc,
            value: current.to_string(),
        })
    }

    fn lex_lt(&mut self) -> Result<Token, LexError> {
        let loc = self.current_location();
        let current = self.get_char().ok_or(LexError::UnexpectedEof {
            context: "'<'",
            location: loc.clone(),
        })?;

        if self.peek(1) == Some('=') {
            let next = self.peek(1).unwrap();
            self.advance();
            self.advance();
            return Ok(Token {
                ttype: TokenType::LessThanEquals,
                location: loc,
                value: format!("{}{}", current, next),
            });
        }

        self.advance();
        Ok(Token {
            ttype: TokenType::LessThan,
            location: loc,
            value: current.to_string(),
        })
    }

    fn lex_not(&mut self) -> Result<Token, LexError> {
        let loc = self.current_location();
        let current = self.get_char().ok_or(LexError::UnexpectedEof {
            context: "'!'",
            location: loc.clone(),
        })?;

        if self.peek(1) == Some('=') {
            let next = self.peek(1).unwrap();
            self.advance();
            self.advance();
            return Ok(Token {
                ttype: TokenType::NotEquals,
                location: loc,
                value: format!("{}{}", current, next),
            });
        }

        self.advance();
        Ok(Token {
            ttype: TokenType::Not,
            location: loc,
            value: current.to_string(),
        })
    }

    fn lex_assign(&mut self) -> Result<Token, LexError> {
        let loc = self.current_location();
        let current = self.get_char().ok_or(LexError::UnexpectedEof {
            context: "'='",
            location: loc.clone(),
        })?;

        if self.peek(1) == Some('=') {
            let next = self.peek(1).unwrap();
            self.advance();
            self.advance();
            return Ok(Token {
                ttype: TokenType::Equals,
                location: loc,
                value: format!("{}{}", current, next),
            });
        }

        self.advance();
        Ok(Token {
            ttype: TokenType::Assign,
            location: loc,
            value: current.to_string(),
        })
    }

    fn lex_char(&mut self) -> Result<Token, LexError> {
        let loc = self.current_location();

        self.advance(); // consume opening '

        let next_char = self.get_char();
        self.advance();

        let final_char = match next_char {
            Some('\\') => {
                let escape_type = self.get_char();
                self.advance();

                match escape_type {
                    Some('n') => '\n',
                    Some('t') => '\t',
                    Some('r') => '\r',
                    Some('\\') => '\\',
                    Some('\'') => '\'',
                    Some('0') => '\0',
                    Some(other) => {
                        return Err(LexError::UnknownEscapeSequence {
                            ch: other,
                            location: loc,
                        });
                    }
                    None => {
                        return Err(LexError::UnexpectedEof {
                            context: "escape sequence",
                            location: loc,
                        });
                    }
                }
            }
            Some(ch) => ch,
            None => {
                return Err(LexError::UnexpectedEof {
                    context: "character literal",
                    location: loc,
                });
            }
        };

        if self.get_char() != Some('\'') {
            return Err(LexError::UnterminatedCharLiteral { location: loc });
        }
        self.advance();

        Ok(Token {
            ttype: TokenType::CharLiteral,
            location: loc,
            value: final_char.to_string(),
        })
    }

    fn lex_string(&mut self) -> Token {
        // Unchanged — no panics here; an unterminated string just
        // runs to EOF and returns whatever was collected. If you want
        // that treated as an error too, this needs the same Result
        // treatment as lex_char.
        let loc = self.current_location();
        let mut string: Vec<char> = Vec::new();
        self.advance(); // "

        while let Some(ch) = self.get_char() {
            if ch != '"' {
                string.push(ch);
                self.advance();
            } else {
                self.advance();
                break;
            }
        }

        let value: String = string.into_iter().collect();
        Token {
            ttype: TokenType::StringLiteral,
            location: loc,
            value,
        }
    }

    fn lex_numeric(&mut self) -> Token {
        // unchanged, no panics
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

        Token {
            ttype: TokenType::IntLiteral,
            location: loc,
            value: numstring.into_iter().collect(),
        }
    }

    fn lex_identifier_and_keyword(&mut self) -> Token {
        // unchanged, no panics — keyword matching is exhaustive-safe
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
        let ttype = match value.as_str() {
            "var" => TokenType::VarKeyword,
            "if" => TokenType::IfKeyword,
            "else" => TokenType::ElseKeyword,
            "while" => TokenType::WhileKeyword,
            "fn" => TokenType::FnKeyword,
            "pub" => TokenType::PubKeyword,
            "use" => TokenType::UseKeyword,
            "for" => TokenType::ForKeyword,
            "return" => TokenType::ReturnKeyword,
            "extern" => TokenType::ExternKeyword,
            "true" => TokenType::True,
            "false" => TokenType::False,
            "struct" => TokenType::StructKeyword,
            "sizeof" => TokenType::SizeOfKeyword,
            _ => TokenType::Identifier,
        };

        Token { ttype, location: loc, value }
    }
}