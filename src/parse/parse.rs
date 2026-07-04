use crate::lexing::tokens::{Token, TokenType};
use crate::parse::ast::{BinaryOp, Expr, Program, Stmt, UnaryOp, Literal};

pub struct Parser {
    pub tokens: Vec<Token>, // tokenised source
    pub token_idx: usize,
    pub ast: Program
}
impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, token_idx: 0 as usize, ast: Program { statements: Vec::new() } }
    }

    pub fn advance(&mut self) {
        self.token_idx += 1;
    }

    pub fn peek(&self, offset: i32) -> Token {
        let true_offset = self.token_idx + offset as usize;

        if true_offset < self.tokens.len() {
            return self.tokens[true_offset].clone()
        }

        panic!("how did we get here?")
    }

    pub fn get_token(&self) -> Token {
        self.peek(0)
    }

    pub fn expect(&self, ttype: TokenType) -> Token {
        let tk = self.get_token();

        if tk.ttype == ttype {
            tk
        } else {
            panic!(
                "Expected {:?}, found {:?}",
                ttype, tk.ttype
            );
        }
    }
    
    pub fn parse(&mut self) {
        let mut statements = Vec::new();

        while self.token_idx < self.tokens.len() - 1 {
            statements.push(self.parse_statement());
        }

        self.ast = Program { statements }
    }

    fn parse_block(&mut self) -> Vec<Stmt> {
        let mut statements = Vec::new();

        while self.get_token().ttype != TokenType::RBrace {
            statements.push(self.parse_statement());
        }

        self.expect(TokenType::RBrace);
        self.advance();

        statements
    }

    pub fn parse_statement(&mut self) -> Stmt {
        let tk = self.get_token();

        match tk.ttype {
            TokenType::VarKeyword => {return self.parse_assignment()}
            TokenType::Ifkeyword => {return self.parse_if()}

            _ => {return Stmt::Expr(self.parse_expr())}
        }
    }

    pub fn parse_assignment(&mut self) -> Stmt {
        self.advance(); // skip var keyword

        let ident = self.expect(TokenType::Identifier);
        self.advance();
        let _ = self.expect(TokenType::Assign);
        self.advance();
        let expr = self.parse_expr();

        Stmt::Assignment {
            ident: ident.value,
            expr
        }
    }

    // if statements will be parenthesised: if (cond) { body }
    pub fn parse_if(&mut self) -> Stmt {
        self.advance(); // skip if keyword
        
        self.expect(TokenType::LParen);
        self.advance();

        let cond = self.parse_expr();

        self.expect(TokenType::RParen);
        self.advance();

        self.expect(TokenType::LBrace);
        self.advance();

        let body = self.parse_block();

        Stmt::If{
            cond,
            then_branch: body
        }
    }    

    // Order of precedence:
    // 1. "paren" Parentheses
    // 2. "unary" Unary Operators
    // 3. "muldiv" Division, Multiplication, Modulo
    // 4. "addsub" Addition, Subtraction
    pub fn parse_expr(&mut self) -> Expr {
        self.parse_addsub()
    }

    // parse "addsub" precedence
    pub fn parse_addsub(&mut self) -> Expr {
        let mut left = self.parse_muldiv();

        while matches!(self.get_token().ttype, TokenType::Add | TokenType::Minus) {
            let op = match self.get_token().ttype {
                TokenType::Add => BinaryOp::Add,
                TokenType::Minus => BinaryOp::Sub,
                _ => unreachable!(),
            };

            self.advance(); // consume operator

            let right = self.parse_muldiv();

            left = Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }

        left
    }

    // parse "muldiv" precedence
    pub fn parse_muldiv(&mut self) -> Expr {
        let mut left = self.parse_unary();

        while matches!(
            self.get_token().ttype,
            TokenType::Multiply | TokenType::Divide | TokenType::Modulo
        ) {
            let op = match self.get_token().ttype {
                TokenType::Multiply => BinaryOp::Mul,
                TokenType::Divide => BinaryOp::Div,
                TokenType::Modulo => BinaryOp::Mod,
                _ => unreachable!(),
            };

            self.advance();

            let right = self.parse_unary();

            left = Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }

        left
    }

    // parse "unary" precedence
    pub fn parse_unary(&mut self) -> Expr {
        match self.get_token().ttype {
            TokenType::Add => {
                self.advance();
                Expr::Unary {
                    op: UnaryOp::Positive,
                    expr: Box::new(self.parse_unary()),
                }
            }
            TokenType::Minus => {
                self.advance();
                Expr::Unary {
                    op: UnaryOp::Negative,
                    expr: Box::new(self.parse_unary()),
                }
            }
            _ => self.parse_primary(),
        }
    }

    // parse "parentheses" precedence + atoms
    pub fn parse_primary(&mut self) -> Expr {
        let tk = self.get_token();

        match tk.ttype {
            TokenType::IntLiteral => {
                self.advance();
                Expr::Literal(Literal::Int(tk.value))
            }

            TokenType::LParen => {
                self.advance();
                let e = self.parse_expr();
                self.expect(TokenType::RParen);
                self.advance();
                e
            }

            _ => todo!(),
        }
    }
}