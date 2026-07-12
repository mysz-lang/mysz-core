use crate::lexing::lexing::{Token, TokenType};
use crate::parse::parsing::{
    BinaryOp, Expr, ExprKind, Identifier, Literal, Parameter, ParserError, ParserErrorType,
    Program, Stmt, Type, UnaryOp,
};
use crate::utils::toident::to_ident;

pub struct Parser {
    pub tokens: Vec<Token>,
    pub token_idx: usize,
    pub ast: Program,
    pub parser_errs: Vec<ParserError>,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            token_idx: 0,
            ast: Program {
                statements: Vec::new(),
            },
            parser_errs: Vec::new(),
        }
    }

    fn eof(&self) -> bool {
        self.token_idx >= self.tokens.len()
    }

    fn current(&self) -> Option<&Token> {
        self.tokens.get(self.token_idx)
    }

    fn throw(&mut self, etype: ParserErrorType, message: String) -> Token {
        let location = self
            .tokens
            .get(self.token_idx.saturating_sub(1))
            .map(|t| t.location.clone());

        self.parser_errs.push(ParserError {
            etype,
            message,
            location: location.clone().unwrap(),
        });

        Token {
            ttype: TokenType::Niltoken,
            location: location.unwrap(),
            value: "ERROR".to_string(),
        }
    }

    fn advance(&mut self) {
        if self.token_idx < self.tokens.len() {
            self.token_idx += 1;
        }
    }

    fn get_token(&self) -> Option<&Token> {
        self.current()
    }

    fn expect(&mut self, ttype: TokenType) -> Option<Token> {
        let tk = self.get_token()?.clone();

        if tk.ttype == ttype {
            self.advance();
            Some(tk)
        } else {
            self.throw(
                ParserErrorType::UnexpectedTokenTypeError,
                format!(
                    "Expected {:?}, found {:?} '{:?}'",
                    ttype, tk.ttype, tk.value
                ),
            );
            None
        }
    }

    pub fn parse(&mut self) {
        let mut statements = Vec::new();

        while !self.eof() {
            if let Some(stmt) = self.parse_statement(true) {
                statements.push(stmt);
            } else {
                self.advance();
            }
        }

        self.ast = Program { statements };
    }

    fn parse_generic_params(&mut self) -> Vec<String> {
        let mut params = Vec::new();
        if matches!(
            self.get_token().map(|t| &t.ttype),
            Some(TokenType::LessThan)
        ) {
            self.advance(); // consume '<'
            loop {
                if let Some(tk) = self.expect(TokenType::Identifier) {
                    params.push(tk.value);
                } else {
                    break;
                }

                if matches!(self.get_token().map(|t| &t.ttype), Some(TokenType::Comma)) {
                    self.advance();
                } else {
                    break;
                }
            }
            self.expect(TokenType::GreaterThan);
        }
        params
    }

    fn parse_generic_args(&mut self) -> Vec<Type> {
        let mut args = Vec::new();
        if matches!(
            self.get_token().map(|t| &t.ttype),
            Some(TokenType::LessThan)
        ) {
            self.advance(); // consume '<'
            loop {
                if let Some(ty) = self.parse_type() {
                    args.push(ty);
                } else {
                    break;
                }

                if matches!(self.get_token().map(|t| &t.ttype), Some(TokenType::Comma)) {
                    self.advance();
                } else {
                    break;
                }
            }
            self.expect(TokenType::GreaterThan);
        }
        args
    }

    fn parse_type(&mut self) -> Option<Type> {
        let tk = self.get_token()?.clone();

        match tk.ttype {
            TokenType::LBracket => {
                self.advance();
                let element_type = self.parse_type()?;
                self.expect(TokenType::SemiColon)?;

                let size_tk = self.expect(TokenType::IntLiteral)?;
                let size = size_tk.value.parse::<usize>().unwrap();
                self.expect(TokenType::RBracket)?;

                Some(Type::Array {
                    element_type: Box::new(element_type),
                    size,
                })
            }

            TokenType::Identifier => match tk.value.as_str() {
                "int" => {
                    self.advance();
                    Some(Type::Int)
                }
                "bool" => {
                    self.advance();
                    Some(Type::Bool)
                }
                "str" => {
                    self.advance();
                    Some(Type::Str)
                }
                "void" => {
                    self.advance();
                    Some(Type::Void)
                }
                "ptr" => {
                    self.advance();
                    self.expect(TokenType::LessThan)?;
                    let inner = self.parse_type()?;
                    self.expect(TokenType::GreaterThan)?;
                    Some(Type::Ptr(Box::new(inner)))
                }
                "any" => {
                    self.advance();
                    Some(Type::Any)
                }
                "char" => {
                    self.advance();
                    Some(Type::Char)
                }
                other => {
                    let struct_name = other.to_string();
                    self.advance();

                    if matches!(
                        self.get_token().map(|t| &t.ttype),
                        Some(TokenType::LessThan)
                    ) {
                        let args = self.parse_generic_args();
                        Some(Type::GenericInstance {
                            name: struct_name,
                            args,
                        })
                    } else {
                        Some(Type::Struct(struct_name))
                    }
                }
            },
            _ => {
                self.throw(
                    ParserErrorType::UnexpectedTokenTypeError,
                    format!("Expected type metadata, found {:?}", tk.ttype),
                );
                None
            }
        }
    }

    fn parse_block(&mut self) -> Vec<Stmt> {
        let mut statements = Vec::new();

        while !self.eof() && !matches!(self.get_token().map(|t| &t.ttype), Some(TokenType::RBrace))
        {
            if let Some(stmt) = self.parse_statement(true) {
                statements.push(stmt);
            } else {
                self.advance();
            }
        }

        self.expect(TokenType::RBrace);
        statements
    }

    fn parse_statement(&mut self, semi_colon: bool) -> Option<Stmt> {
        let tk = self.get_token()?.clone();

        let stmt = match tk.ttype {
            TokenType::VarKeyword => self.parse_assignment(),
            TokenType::StructKeyword => self.parse_struct(),
            TokenType::IfKeyword => self.parse_if(),
            TokenType::WhileKeyword => self.parse_while(),
            TokenType::FnKeyword => self.parse_function(),
            TokenType::ForKeyword => self.parse_for(),
            TokenType::ReturnKeyword => self.parse_return(),
            TokenType::BreakKeyword => self.parse_break(),
            TokenType::UseKeyword => self.parse_import(),
            TokenType::ExternKeyword => self.parse_extern(),
            TokenType::Identifier => self.parse_ident(),
            TokenType::Star => {
                let pointer_expr = self.parse_unary()?;

                self.expect(TokenType::Assign)?;
                let value_expr = self.parse_expr()?;

                Some(Stmt::DerefReassignment {
                    target: pointer_expr,
                    expr: value_expr,
                })
            }
            _ => self.parse_expr().map(Stmt::Expr),
        };

        if semi_colon {
            if self.expect(TokenType::SemiColon).is_none() {
                self.throw(
                    ParserErrorType::MalformedStatementError,
                    format!("Statement did not finish with semicolon ';'"),
                );
            }
        }
        stmt
    }

    fn parse_import(&mut self) -> Option<Stmt> {
        self.advance();
        let mut path = Vec::new();

        loop {
            let ident = match self.get_token().map(|t| &t.ttype) {
                Some(TokenType::Identifier) => {
                    let token = self.get_token().cloned()?;
                    self.advance();
                    token.value
                }
                other => {
                    self.throw(
                        ParserErrorType::UnexpectedTokenTypeError,
                        format!("Expected identifier in use path, found {:?}", other),
                    );
                    return None;
                }
            };
            path.push(ident);

            match self.get_token().map(|t| &t.ttype) {
                Some(TokenType::DoubleColon) => {
                    self.advance();
                }
                Some(TokenType::SemiColon) => {
                    break;
                }
                other => {
                    self.throw(
                        ParserErrorType::UnexpectedTokenTypeError,
                        format!("Expected '::' or ';', found {:?}", other),
                    );
                    return None;
                }
            }
        }

        Some(Stmt::Use { path })
    }

    fn parse_extern(&mut self) -> Option<Stmt> {
        self.advance(); // consume 'extern'
        self.expect(TokenType::FnKeyword)?;

        let ident = self.expect(TokenType::Identifier)?;

        let generic_params = self.parse_generic_params();

        self.expect(TokenType::LParen)?;
        let params = self.parse_params(TokenType::RParen);

        let rttype = if matches!(self.get_token().map(|t| &t.ttype), Some(TokenType::Colon)) {
            self.advance();
            self.parse_type()
        } else {
            None
        };

        Some(Stmt::Extern {
            name: to_ident(Some(ident))?,
            rttype,
            generic_params,
            params,
        })
    }

    fn parse_ident(&mut self) -> Option<Stmt> {
        let lhs_expr = self.parse_postfix()?;

        if matches!(self.get_token().map(|t| &t.ttype), Some(TokenType::Assign)) {
            self.advance();
            let value_expr = self.parse_expr()?;

            if matches!(
                lhs_expr.kind,
                ExprKind::Index { .. } | ExprKind::Field { .. }
            ) {
                return Some(Stmt::DerefReassignment {
                    target: lhs_expr,
                    expr: value_expr,
                });
            } else if let ExprKind::Identifier(name) = lhs_expr.kind {
                return Some(Stmt::Reassignment {
                    ident: Identifier {
                        value: name,
                        location: lhs_expr.span,
                    },
                    expr: value_expr,
                });
            }
        }

        Some(Stmt::Expr(lhs_expr))
    }

    fn parse_params(&mut self, ending: TokenType) -> Vec<Parameter> {
        let mut params = Vec::new();

        if self.get_token().map(|t| &t.ttype) == Some(&ending) {
            self.advance();
            return params;
        }

        loop {
            let name = match self
                .get_token()
                .cloned()
                .and_then(|token| to_ident(Some(token)))
            {
                Some(ident) => {
                    self.advance();
                    ident
                }
                other => {
                    self.throw(
                        ParserErrorType::UnexpectedTokenTypeError,
                        format!("Expected parameter name, found {:?}", other),
                    );
                    break;
                }
            };

            let ptype = if self.get_token().map(|t| &t.ttype) == Some(&TokenType::Colon) {
                self.advance();
                self.parse_type()
            } else {
                None
            };

            params.push(Parameter { name, ptype });

            match self.get_token().map(|t| &t.ttype) {
                Some(TokenType::Comma) => {
                    self.advance();
                }
                Some(ttype) if ttype == &ending => {
                    self.advance();
                    break;
                }
                other => {
                    self.throw(
                        ParserErrorType::UnexpectedTokenTypeError,
                        format!("Expected ',' or {:?}, found {:?}", ending, other),
                    );
                    break;
                }
            }
        }

        params
    }

    fn parse_args(&mut self) -> Vec<Expr> {
        let mut args = Vec::new();

        if matches!(self.get_token().map(|t| &t.ttype), Some(TokenType::RParen)) {
            self.advance();
            return args;
        }

        loop {
            match self.parse_expr() {
                Some(expr) => args.push(expr),
                None => break,
            }

            match self.get_token().map(|t| &t.ttype) {
                Some(TokenType::Comma) => {
                    self.advance();
                }
                Some(TokenType::RParen) => {
                    self.advance();
                    break;
                }
                other => {
                    self.throw(
                        ParserErrorType::UnexpectedTokenTypeError,
                        format!("Expected ',' or ')', found {:?}", other),
                    );
                    break;
                }
            }
        }

        args
    }

    fn parse_function(&mut self) -> Option<Stmt> {
        self.advance();

        let public = match self.get_token()?.ttype {
            TokenType::PubKeyword => {
                self.advance();
                true
            }
            _ => false,
        };

        let ident = self.expect(TokenType::Identifier)?;

        let generic_params = self.parse_generic_params();

        self.expect(TokenType::LParen)?;
        let params = self.parse_params(TokenType::RParen);

        let mut rttype = None;
        if matches!(self.get_token().map(|t| &t.ttype), Some(TokenType::Colon)) {
            self.advance();
            rttype = self.parse_type();
        }

        self.expect(TokenType::LBrace)?;
        let body = self.parse_block();

        Some(Stmt::Function {
            name: Identifier {
                value: ident.value,
                location: ident.location,
            },
            public,
            rttype,
            generic_params,
            params,
            body,
        })
    }

    fn parse_break(&mut self) -> Option<Stmt> {
        let tk = self.get_token()?.clone();
        self.advance();

        Some(Stmt::Break {
            location: tk.location,
        })
    }

    fn parse_return(&mut self) -> Option<Stmt> {
        let tk = self.get_token()?.clone();
        self.advance();

        let expr = match self.get_token().map(|t| &t.ttype) {
            Some(TokenType::SemiColon | TokenType::RBrace) => None,
            _ => Some(self.parse_expr()?),
        };

        Some(Stmt::Return {
            value: expr,
            span: tk.location,
        })
    }

    fn parse_assignment(&mut self) -> Option<Stmt> {
        self.advance();

        let ident = self.expect(TokenType::Identifier)?;
        let ident_loc = ident.location.clone();

        let mut vtype = None;

        if matches!(self.get_token().map(|t| &t.ttype), Some(TokenType::Colon)) {
            self.advance();
            vtype = self.parse_type();
        }

        let value = if matches!(self.get_token().map(|t| &t.ttype), Some(TokenType::Assign)) {
            self.advance();
            Some(self.parse_expr()?)
        } else {
            None
        };

        Some(Stmt::Assignment {
            ident: Identifier {
                value: ident.value,
                location: ident_loc,
            },
            vtype,
            expr: value,
        })
    }

    fn parse_for(&mut self) -> Option<Stmt> {
        self.advance();

        self.expect(TokenType::LParen)?;
        let init = self.parse_statement(false)?;
        self.expect(TokenType::SemiColon)?;
        let cond = self.parse_expr()?;
        self.expect(TokenType::SemiColon)?;
        let step = self.parse_statement(false)?;
        self.expect(TokenType::RParen)?;

        self.expect(TokenType::LBrace)?;
        let body = self.parse_block();

        Some(Stmt::For {
            init: Box::new(init),
            cond,
            step: Box::new(step),
            body,
        })
    }

    fn parse_while(&mut self) -> Option<Stmt> {
        self.advance();

        self.expect(TokenType::LParen)?;
        let cond = self.parse_expr()?;
        self.expect(TokenType::RParen)?;

        self.expect(TokenType::LBrace)?;
        let body = self.parse_block();

        Some(Stmt::While { cond, body })
    }

    fn parse_struct(&mut self) -> Option<Stmt> {
        self.advance();

        let ident = self.expect(TokenType::Identifier)?;

        let generic_params = self.parse_generic_params();

        self.expect(TokenType::LBrace)?;
        let mut fields = Vec::new();

        while !self.eof() && !matches!(self.get_token().map(|t| &t.ttype), Some(TokenType::RBrace))
        {
            let name = to_ident(self.get_token().cloned())?;
            self.advance();
            self.expect(TokenType::Colon)?;

            let ptype = self.parse_type();
            fields.push(Parameter { name, ptype });

            if matches!(self.get_token().map(|t| &t.ttype), Some(TokenType::Comma)) {
                self.advance();
            } else if !matches!(self.get_token().map(|t| &t.ttype), Some(TokenType::RBrace)) {
                self.throw(
                    ParserErrorType::UnexpectedTokenTypeError,
                    "Expected ',' or '}' after struct field".to_string(),
                );
                return None;
            }
        }

        self.expect(TokenType::RBrace)?;

        Some(Stmt::Struct {
            name: to_ident(Some(ident))?,
            generic_params,
            fields,
        })
    }

    fn parse_if(&mut self) -> Option<Stmt> {
        self.advance();

        self.expect(TokenType::LParen)?;
        let cond = self.parse_expr()?;
        self.expect(TokenType::RParen)?;

        self.expect(TokenType::LBrace)?;
        let body = self.parse_block();

        let else_branch = if matches!(
            self.get_token().map(|t| &t.ttype),
            Some(TokenType::ElseKeyword)
        ) {
            self.advance();
            self.expect(TokenType::LBrace);
            Some(self.parse_block())
        } else {
            None
        };

        Some(Stmt::If {
            cond,
            then_branch: body,
            else_branch,
        })
    }

    fn parse_array_literal(&mut self) -> Option<Expr> {
        let open_bracket = self.get_token()?.clone();
        self.advance();

        let mut elements = Vec::new();

        if matches!(
            self.get_token().map(|t| &t.ttype),
            Some(TokenType::RBracket)
        ) {
            self.advance();
            return Some(Expr {
                kind: ExprKind::Literal(Literal::Arr { elements }),
                span: open_bracket.location,
            });
        }

        loop {
            let expr = self.parse_expr()?;
            elements.push(expr);

            match self.get_token().map(|t| &t.ttype) {
                Some(TokenType::Comma) => {
                    self.advance();
                }
                Some(TokenType::RBracket) => {
                    self.advance();
                    break;
                }
                other => {
                    self.throw(
                        ParserErrorType::UnexpectedTokenTypeError,
                        format!("Expected ',' or ']', found {:?}", other),
                    );
                    return None;
                }
            }
        }

        Some(Expr {
            kind: ExprKind::Literal(Literal::Arr { elements }),
            span: open_bracket.location,
        })
    }

    fn parse_expr(&mut self) -> Option<Expr> {
        match self.get_token().map(|t| &t.ttype) {
            Some(TokenType::RParen | TokenType::RBrace | TokenType::SemiColon) => {
                return None;
            }
            _ => {}
        }
        self.parse_equality()
    }

    fn parse_equality(&mut self) -> Option<Expr> {
        let mut left = self.parse_comparison()?;

        while matches!(
            self.get_token().map(|t| t.ttype.clone()),
            Some(TokenType::Equals | TokenType::NotEquals)
        ) {
            let op_token = self.get_token()?.clone();
            self.advance();

            let op = match op_token.ttype {
                TokenType::Equals => BinaryOp::Eq,
                TokenType::NotEquals => BinaryOp::NEq,
                _ => unreachable!(),
            };

            let right = self.parse_comparison()?;

            left = Expr {
                kind: ExprKind::Binary {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span: op_token.location,
            };
        }

        Some(left)
    }

    fn parse_comparison(&mut self) -> Option<Expr> {
        let mut left = self.parse_addsub()?;

        while matches!(
            self.get_token().map(|t| t.ttype.clone()),
            Some(
                TokenType::LessThan
                    | TokenType::LessThanEquals
                    | TokenType::GreaterThan
                    | TokenType::GreaterThanEquals
            )
        ) {
            let op_token = self.get_token()?.clone();
            self.advance();

            let op = match op_token.ttype {
                TokenType::LessThan => BinaryOp::Lt,
                TokenType::LessThanEquals => BinaryOp::LtE,
                TokenType::GreaterThan => BinaryOp::Gt,
                TokenType::GreaterThanEquals => BinaryOp::GtE,
                _ => unreachable!(),
            };

            let right = self.parse_addsub()?;

            left = Expr {
                kind: ExprKind::Binary {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span: op_token.location,
            };
        }

        Some(left)
    }

    fn parse_addsub(&mut self) -> Option<Expr> {
        let mut left = self.parse_muldiv()?;

        while matches!(
            self.get_token().map(|t| t.ttype.clone()),
            Some(TokenType::Add | TokenType::Minus)
        ) {
            let op_token = self.get_token()?.clone();

            let op = match op_token.ttype {
                TokenType::Add => BinaryOp::Add,
                TokenType::Minus => BinaryOp::Sub,
                _ => unreachable!(),
            };

            self.advance();
            let right = self.parse_muldiv()?;
            let span = left.span.clone();

            left = Expr {
                kind: ExprKind::Binary {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span,
            };
        }

        Some(left)
    }

    fn parse_muldiv(&mut self) -> Option<Expr> {
        let mut left = self.parse_unary()?;

        while matches!(
            self.get_token().map(|t| t.ttype.clone()),
            Some(TokenType::Multiply | TokenType::Divide | TokenType::Modulo)
        ) {
            let op_token = self.get_token()?.clone();

            let op = match op_token.ttype {
                TokenType::Multiply => BinaryOp::Mul,
                TokenType::Divide => BinaryOp::Div,
                TokenType::Modulo => BinaryOp::Mod,
                _ => unreachable!(),
            };

            self.advance();
            let right = self.parse_unary()?;
            let span = left.span.clone();

            left = Expr {
                kind: ExprKind::Binary {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span,
            };
        }

        Some(left)
    }

    fn parse_unary(&mut self) -> Option<Expr> {
        let tk = self.get_token()?.clone();

        match tk.ttype {
            TokenType::Add => {
                self.advance();
                let expr = self.parse_unary()?;
                Some(Expr {
                    kind: ExprKind::Unary {
                        op: UnaryOp::Positive,
                        expr: Box::new(expr),
                    },
                    span: tk.location,
                })
            }
            TokenType::Minus => {
                self.advance();
                let expr = self.parse_unary()?;
                Some(Expr {
                    kind: ExprKind::Unary {
                        op: UnaryOp::Negative,
                        expr: Box::new(expr),
                    },
                    span: tk.location,
                })
            }
            TokenType::Ampersand => {
                self.advance();
                let expr = self.parse_unary()?;
                Some(Expr {
                    kind: ExprKind::Unary {
                        op: UnaryOp::AddressOf,
                        expr: Box::new(expr),
                    },
                    span: tk.location,
                })
            }
            TokenType::Star => {
                self.advance();
                let expr = self.parse_unary()?;
                Some(Expr {
                    kind: ExprKind::Unary {
                        op: UnaryOp::Deref,
                        expr: Box::new(expr),
                    },
                    span: tk.location,
                })
            }
            TokenType::Not => {
                self.advance();
                let expr = self.parse_unary()?;
                Some(Expr {
                    kind: ExprKind::Unary {
                        op: UnaryOp::Not,
                        expr: Box::new(expr),
                    },
                    span: tk.location,
                })
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Option<Expr> {
        let mut expr = self.parse_primary()?;

        loop {
            match self.get_token().map(|t| &t.ttype) {
                Some(TokenType::LBracket) => {
                    self.advance();
                    let index_expr = self.parse_expr()?;
                    let close_tk = self.expect(TokenType::RBracket)?;
                    expr = Expr {
                        kind: ExprKind::Index {
                            base: Box::new(expr),
                            index: Box::new(index_expr),
                        },
                        span: close_tk.location,
                    };
                }
                Some(TokenType::Period) => {
                    self.advance();
                    let field_tk = self.expect(TokenType::Identifier)?;
                    let loc = field_tk.location.clone();
                    expr = Expr {
                        kind: ExprKind::Field {
                            base: Box::new(expr),
                            field: field_tk.value,
                        },
                        span: loc,
                    };
                }
                Some(TokenType::LParen) => {
                    if let ExprKind::Identifier(name) = &expr.kind {
                        let callee_loc = expr.span.clone();
                        self.advance();
                        let args = self.parse_args();
                        expr = Expr {
                            kind: ExprKind::Call {
                                callee: Identifier {
                                    value: name.clone(),
                                    location: callee_loc.clone(),
                                },
                                generic_args: Vec::new(),
                                args,
                            },
                            span: callee_loc,
                        };
                    } else {
                        self.throw(
                            ParserErrorType::UnexpectedTokenTypeError,
                            "Expected function name before parenthesis".to_string(),
                        );
                        return None;
                    }
                }
                Some(TokenType::DoubleColon) => {
                    let next_tk = self.tokens.get(self.token_idx + 1);
                    if matches!(next_tk.map(|t| &t.ttype), Some(TokenType::LessThan)) {
                        self.advance(); // consume '::'
                        let generic_args = self.parse_generic_args();

                        self.expect(TokenType::LParen)?;
                        if let ExprKind::Identifier(name) = &expr.kind {
                            let callee_loc = expr.span.clone();
                            let args = self.parse_args();
                            expr = Expr {
                                kind: ExprKind::Call {
                                    callee: Identifier {
                                        value: name.clone(),
                                        location: callee_loc.clone(),
                                    },
                                    generic_args,
                                    args,
                                },
                                span: callee_loc,
                            };
                        } else {
                            return None;
                        }
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }
        Some(expr)
    }

    fn parse_primary(&mut self) -> Option<Expr> {
        let tk = self.get_token()?.clone();

        match tk.ttype {
            TokenType::IntLiteral => {
                self.advance();
                let value = tk.value.parse::<i64>().unwrap();
                Some(Expr {
                    kind: ExprKind::Literal(Literal::Int(value)),
                    span: tk.location,
                })
            }

            TokenType::True => {
                self.advance();
                let value = true;
                Some(Expr {
                    kind: ExprKind::Literal(Literal::Bool(value)),
                    span: tk.location,
                })
            }
            TokenType::False => {
                self.advance();
                let value = false;
                Some(Expr {
                    kind: ExprKind::Literal(Literal::Bool(value)),
                    span: tk.location,
                })
            }

            TokenType::StringLiteral => {
                self.advance();
                Some(Expr {
                    kind: ExprKind::Literal(Literal::String(tk.value)),
                    span: tk.location,
                })
            }
            TokenType::CharLiteral => {
                self.advance();
                let value = tk.value.chars().next().unwrap();
                Some(Expr {
                    kind: ExprKind::Literal(Literal::Char(value)),
                    span: tk.location,
                })
            }
            TokenType::SizeOfKeyword => {
                let start_tk = self.get_token().cloned()?;
                self.advance(); // consume 'sizeof'

                self.expect(TokenType::LParen)?;
                let target_type = self.parse_type()?;
                self.expect(TokenType::RParen)?;

                Some(Expr {
                    kind: ExprKind::Sizeof { ty: target_type },
                    span: start_tk.location,
                })
            }
            TokenType::Identifier => {
                let id_tk = self.get_token()?.clone();
                self.advance();

                if matches!(self.get_token().map(|t| &t.ttype), Some(TokenType::LBrace)) {
                    self.advance(); // consume '{'
                    let mut fields = Vec::new();

                    if !matches!(self.get_token().map(|t| &t.ttype), Some(TokenType::RBrace)) {
                        loop {
                            let field_name = self.expect(TokenType::Identifier)?.value;
                            self.expect(TokenType::Colon)?;
                            let value_expr = self.parse_expr()?;
                            fields.push((field_name, value_expr));

                            match self.get_token().map(|t| &t.ttype) {
                                Some(TokenType::Comma) => {
                                    self.advance();
                                }
                                Some(TokenType::RBrace) => break,
                                _ => {
                                    self.throw(
                                        ParserErrorType::UnexpectedTokenTypeError,
                                        "Expected ',' or '}' in struct initializer".to_string(),
                                    );
                                    return None;
                                }
                            }
                        }
                    }

                    let end_tk = self.expect(TokenType::RBrace)?;
                    Some(Expr {
                        kind: ExprKind::StructLiteral {
                            struct_name: id_tk.value,
                            fields,
                        },
                        span: end_tk.location,
                    })
                } else {
                    Some(Expr {
                        kind: ExprKind::Identifier(id_tk.value),
                        span: id_tk.location,
                    })
                }
            }
            TokenType::LBracket => self.parse_array_literal(),

            TokenType::LParen => {
                self.advance();
                let expr = self.parse_expr()?;
                self.expect(TokenType::RParen)?;
                Some(expr)
            }

            _ => {
                self.throw(
                    ParserErrorType::UnexpectedTokenTypeError,
                    format!("Unexpected token in expression: {:?}", tk.ttype),
                );
                None
            }
        }
    }
}
