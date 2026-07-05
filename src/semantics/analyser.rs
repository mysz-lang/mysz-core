use std::collections::HashMap;

use crate::parse::parsing::*;
use crate::semantics::analysis::{Scope, Symbol, Type, FunctionSignature};
use crate::utils::location::Location;


pub struct Analyser {
    pub scopes: Vec<Scope>,
    pub current_scope: usize,
    pub functions: HashMap<String, FunctionSignature>,
    pub types: HashMap<String, Type>,
    current_return_type: Option<Type>,
}
impl Analyser {
    pub fn new() -> Self {
        Self {
            scopes: vec![Scope { symbols: HashMap::new(), parent: None }],
            current_scope: 0,
            functions: HashMap::new(),
            types: HashMap::new(),
            current_return_type: None
        }
    }

    pub fn enter_scope(&mut self) {
        let parent_idx = self.current_scope;
        let new_scope = Scope { symbols: HashMap::new(), parent: Some(parent_idx) };
        self.scopes.push(new_scope);
        self.current_scope = self.scopes.len() - 1;
    }

    pub fn leave_scope(&mut self) {
        let parent = self.scopes[self.current_scope].parent
            .expect("Attempted to leave global scope");

        self.scopes.pop();
        self.current_scope = parent;
    }

    fn declare_variable(&mut self, name: &str, data_type: Type, span: Location) -> Result<(), String> {
        let scope = &mut self.scopes[self.current_scope];
        if scope.symbols.contains_key(name) {
            return Err(format!(
                "Semantic Error [{}]: Variable '{}' already declared in this scope.", 
                span, name
            ));
        }
        scope.symbols.insert(name.to_string(), Symbol { data_type });
        self.types.insert(name.to_string(), data_type);
        Ok(())
    }
    
    fn resolve_variable(&self, name: &str) -> Option<&Symbol> {
        let mut current = self.current_scope;
        loop {
            if let Some(symbol) = self.scopes[current].symbols.get(name) {
                return Some(symbol);
            }
            match self.scopes[current].parent {
                Some(p) => current = p,
                None => break,
            }
        }
        None
    }

    fn declare_function(
        &mut self,
        name: &str,
        param_types: Vec<Type>,
        return_type: Type,
        location: Location,
    ) -> Result<(), String> {
        if let Some(existing) = self.functions.get(name) {
            return Err(format!(
                "Semantic Error [{}]: Function '{}' is already defined at [{}]",
                location, name, existing.location
            ));
        }

        self.functions.insert(
            name.to_string(),
            FunctionSignature { param_types, return_type, location },
        );

        Ok(())
    }

    fn resolve_function(&self, name: &str) -> Option<&FunctionSignature> {
        self.functions.get(name)
    }

    pub fn check_truthiness(&self, ty: Type) -> bool {
        match ty {
            Type::Int => true,
            Type::Bool => true,
            Type::Str => true,
            _ => false,
        }
    }

    pub fn check_expr(&self, expr: &Expr) -> Result<Type, String> {
        match &expr.kind {
            ExprKind::Literal(lit) => match lit {
                Literal::Int(val_str) => {
                    val_str;
                    Ok(Type::Int)
                }
                Literal::String(_) => Ok(Type::Str),
                Literal::Bool(_) => Ok(Type::Bool)
            },
            ExprKind::Identifier(name) => {
                if let Some(symbol) = self.resolve_variable(name) {
                    Ok(symbol.data_type)
                } else {
                    Err(format!("Semantic Error [{}]: Variable '{}' is used before definition.", expr.span, name))
                }
            }
            ExprKind::Call { callee, args } => {
                let sig = self.resolve_function(&callee.value).ok_or_else(|| {
                    format!(
                        "Semantic Error [{}]: Call to undefined function '{}'",
                        callee.location, callee.value
                    )
                })?;

                if args.len() != sig.param_types.len() {
                    return Err(format!(
                        "Type Error [{}]: Function '{}' expects {} argument(s), found {}",
                        expr.span, callee.value, sig.param_types.len(), args.len()
                    ));
                }

                let param_types = sig.param_types.clone();
                let return_type = sig.return_type;

                for (i, (arg, expected)) in args.iter().zip(param_types.iter()).enumerate() {
                    let arg_type = self.check_expr(arg)?;
                    if arg_type != *expected {
                        return Err(format!(
                            "Type Error [{}]: Argument {} to '{}' expects '{:?}', found '{:?}'",
                            arg.span, i + 1, callee.value, expected, arg_type
                        ));
                    }
                }

                Ok(return_type)
            }
            ExprKind::Binary { left, op, right } => {
                let left_type = self.check_expr(left)?;
                let right_type = self.check_expr(right)?;

                match op {
                    BinaryOp::Add => {
                        if left_type == Type::Int && right_type == Type::Int {
                            Ok(Type::Int)
                        } else if left_type == Type::Str && right_type == Type::Str {
                            Ok(Type::Str)
                        } else {
                            Err(format!(
                                "Type Error [{}]: Cannot add type '{:?}' and '{:?}'", 
                                expr.span, left_type, right_type
                            ))
                        }
                    }
                    BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
                        if left_type == Type::Int && right_type == Type::Int {
                            Ok(Type::Int)
                        } else {
                            Err(format!(
                                "Type Error [{}]: Operator '{:?}' expects '{:?}', but found '{:?}' and '{:?}'", 
                                expr.span, op, Type::Int, left_type, right_type
                            ))
                        }
                    }
                    BinaryOp::Eq | BinaryOp::NEq | BinaryOp::Gt | BinaryOp::GtE | BinaryOp::Lt | BinaryOp::LtE => {
                        if left_type == right_type {
                            Ok(left_type)
                        } else {
                            Err(format!(
                                "Type Error [{}]: Operator '{:?}' expects same-type sides, lhs: {:?}, rhs: {:?}",
                                expr.span, op, left_type, right_type
                            ))
                        }
                    }
                }
            }
            ExprKind::Unary { op, expr: sub_expr } => {
                let expr_type = self.check_expr(sub_expr)?;
                match op {
                    UnaryOp::Positive | UnaryOp::Negative => {
                        if expr_type == Type::Int {
                            Ok(Type::Int)
                        } else {
                            Err(format!(
                                "Type Error [{}]: Unary operator expects 'int', found '{:?}'", 
                                expr.span, expr_type
                            ))
                        }
                    }
                }
            }
        }
    }

    pub fn check_stmt(&mut self, stmt: &Stmt) -> Result<(), String> {
        match stmt {
            Stmt::Extern { name, rttype, params } => {
                let return_type = match rttype {
                    Some(rt_ident) => Type::from_ident(rt_ident)?,
                    None => Type::Void,
                };
                let mut param_types = Vec::new();
                for param in params {
                    let ptype = param.ptype.as_ref().ok_or_else(|| {
                        format!(
                            "Type Error [{}]: Parameter '{}' must have an explicit type",
                            param.name.location, param.name.value
                        )
                    })?;
                    param_types.push(Type::from_ident(ptype)?);
                }

                self.declare_function(&name.value, param_types, return_type, name.location.clone())?;

                Ok(())
            }

            Stmt::Assignment { ident, vtype, expr } => {
                let expr_type = self.check_expr(expr)?;

                if let Some(type_ident) = vtype {
                    let explicit_type = Type::from_ident(type_ident)?;
                    if explicit_type != expr_type {
                        return Err(format!(
                            "Type Error [{}]: Variable '{}' declared as '{:?}' but assigned type '{:?}'",
                            expr.span, ident.value, explicit_type, expr_type
                        ));
                    }
                    self.declare_variable(&ident.value, explicit_type, ident.location.clone())?;
                } else {
                    if let Some(existing_symbol) = self.resolve_variable(&ident.value) {
                        if existing_symbol.data_type != expr_type {
                            return Err(format!(
                                "Type Error [{}]: Cannot assign type '{:?}' to variable '{}' of type '{:?}'",
                                expr.span, expr_type, ident.value, existing_symbol.data_type
                            ));
                        }
                    } else {
                        self.declare_variable(&ident.value, expr_type, ident.location.clone())?;
                    }
                }
                Ok(())
            }

            Stmt::Reassignment { ident, expr } => {
                let expr_type = self.check_expr(expr)?;

                // must already exist
                let symbol = self.resolve_variable(&ident.value).ok_or_else(|| {
                    format!(
                        "Semantic Error [{}]: Cannot reassign to undefined variable '{}'",
                        ident.location, ident.value
                    )
                })?;

                // must match type
                if symbol.data_type != expr_type {
                    return Err(format!(
                        "Type Error [{}]: Cannot assign type '{:?}' to variable '{}' of type '{:?}'",
                        expr.span, expr_type, ident.value, symbol.data_type
                    ));
                }

                Ok(())
            }
            Stmt::Expr(expr) => {
                self.check_expr(expr)?;
                Ok(())
            }

            Stmt::While { cond, body } => {
                let cond_type = self.check_expr(cond)?;
                if !self.check_truthiness(cond_type) {
                    return Err(format!(
                        "Type Error [{}]: 'while' condition is not truthy, found '{:?}'",
                        cond.span, cond_type
                    ));
                }

                self.enter_scope();
                for block_stmt in body {
                    self.check_stmt(block_stmt)?;
                }
                self.leave_scope();
                Ok(())
            }

            Stmt::If { cond, then_branch } => {
                let cond_type = self.check_expr(cond)?;
                if !self.check_truthiness(cond_type) {
                    return Err(format!(
                        "Type Error [{}]: 'if' condition is not truthy, found '{:?}'",
                        cond.span, cond_type
                    ));
                }

                self.enter_scope();
                for block_stmt in then_branch {
                    self.check_stmt(block_stmt)?;
                }
                self.leave_scope();
                Ok(())
            }
            Stmt::Function { name, rttype, params, body } => {
                let return_type = match rttype {
                    Some(rt_ident) => Type::from_ident(rt_ident)?,
                    None => Type::Void,
                };

                let mut param_types = Vec::new();
                for param in params {
                    let ptype = param.ptype.as_ref().ok_or_else(|| {
                        format!(
                            "Type Error [{}]: Parameter '{}' must have an explicit type",
                            param.name.location, param.name.value
                        )
                    })?;
                    param_types.push(Type::from_ident(ptype)?);
                }

                self.declare_function(&name.value, param_types.clone(), return_type, name.location.clone())?;

                self.enter_scope();
                for (param, ptype) in params.iter().zip(param_types.into_iter()) {
                    self.declare_variable(&param.name.value, ptype, param.name.location.clone())?;
                }

                let prev_return_type = self.current_return_type.replace(return_type);
                let mut returns = false;
                for block_stmt in body {
                    if let Stmt::Return { .. } = block_stmt {
                        returns = true;
                    }
                    self.check_stmt(block_stmt)?;
                }
                self.current_return_type = prev_return_type;

                if return_type != Type::Void && !returns {
                    return Err(format!(
                        "Type Error [{}]: Function '{}' must return a value of type '{:?}'",
                        name.location, name.value, return_type
                    ));
                }

                self.leave_scope();

                Ok(())
            },
            Stmt::Return { value, span } => {
                let actual_type = match value {
                    Some(e) => self.check_expr(e)?,
                    None => Type::Void,
                };

                let expected = self.current_return_type.ok_or_else(|| {
                    format!("Semantic Error [{}]: 'return' used outside of a function", span)
                })?;

                if actual_type != expected {
                    return Err(format!(
                        "Type Error [{}]: Function expects return type '{:?}', found '{:?}'",
                        span, expected, actual_type
                    ));
                }

                Ok(())
            }
        }
    }

    pub fn analyse(&mut self, program: &Program) -> Result<(), String> {
        for stmt in &program.statements {
            self.check_stmt(stmt)?;
        }
        Ok(())
    }  
}