use std::collections::HashMap;

use crate::parse::parsing::*;
use crate::semantics::analysis::{Scope, Symbol, FunctionSignature};
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
        scope.symbols.insert(name.to_string(), Symbol { data_type: data_type.clone() });
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

    pub fn check_truthiness(&self, ty: &Type) -> bool {
        match ty {
            Type::Int => true,
            Type::Bool => true,
            Type::Str => true,
            _ => false,
        }
    }

    pub fn check_expr(&self, expr: &Expr, expected_type: Option<&Type>) -> Result<Type, String> {
        match &expr.kind {
            ExprKind::Literal(lit) => match lit {
                Literal::Int(_) => Ok(Type::Int),
                Literal::String(_) => Ok(Type::Str),
                Literal::Bool(_) => Ok(Type::Bool),
                Literal::Char(_) => Ok(Type::Char),
                Literal::Arr { elements } => {
                    let element_type = if elements.is_empty() {
                        if let Some(Type::Array { element_type, .. }) = expected_type {
                            *element_type.clone()
                        } else {
                            return Err("Type Error: Cannot infer type of an empty array literal without explicit type context.".to_string());
                        }
                    } else {
                        self.check_expr(&elements[0], None)?
                    };

                    for el in elements {
                        let el_type = self.check_expr(el, Some(&element_type))?;
                        if el_type != element_type {
                            return Err(format!(
                                "Type Error: Heterogeneous arrays are not allowed. Expected {:?}, found {:?}",
                                element_type, el_type
                            ));
                        }
                    }

                    Ok(Type::Array {
                        element_type: Box::new(element_type),
                        size: elements.len(),
                    })
                }
            }

            ExprKind::Index { base, index } => {
                let base_type = self.check_expr(base, None)?;
                let index_type = self.check_expr(index, None)?;

                if index_type != Type::Int {
                    return Err(format!("Array index must be an int."));
                }

                match base_type {
                    Type::Array { element_type, .. } => {
                        Ok(*element_type)
                    }
                    Type::Ptr(inner_type) => {
                        Ok(*inner_type)
                    }
                    _ => Err(format!("Cannot index into non-indexable type '{:?}'", base_type)),
                }
            }
            ExprKind::Identifier(name) => {
                if let Some(symbol) = self.resolve_variable(name) {
                    Ok(symbol.data_type.clone())
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
                let return_type = sig.return_type.clone();

                for (i, (arg, expected)) in args.iter().zip(param_types.iter()).enumerate() {
                    let arg_type = self.check_expr(arg, None)?;
                    match (expected, &arg_type) {
                        (
                            Type::Array {
                                element_type: expected_elem,
                                ..
                            },
                            Type::Array {
                                element_type: actual_elem,
                                ..
                            },
                        ) => {
                            if **expected_elem != Type::Any && **expected_elem != **actual_elem {
                                return Err(format!(
                                    "Type Error [{}]: Argument {} to '{}' expects array of '{:?}', found array of '{:?}'",
                                    arg.span,
                                    i + 1,
                                    callee.value,
                                    expected_elem,
                                    actual_elem,
                                ));
                            }
                        }

                        (Type::Array { .. }, _) => {
                            return Err(format!(
                                "Type Error [{}]: Argument {} to '{}' expects '{:?}', found '{:?}'",
                                arg.span,
                                i + 1,
                                callee.value,
                                expected,
                                arg_type,
                            ));
                        }

                        _ => {}
                    }
                }

                Ok(return_type)
            }
            ExprKind::Binary { left, op, right } => {
                let left_type = self.check_expr(left, None)?;
                let right_type = self.check_expr(right, None)?;

                match op {
                    BinaryOp::Add => {
                        if left_type == Type::Int && right_type == Type::Int {
                            Ok(Type::Int)
                        } else if (left_type == Type::Int || left_type == Type::Any) && 
                            (right_type == Type::Int || right_type == Type::Any) {
                                Ok(Type::Int)
                        } else if left_type == Type::Str && right_type == Type::Str {
                            Ok(Type::Str)
                        } else if (left_type == Type::Str || left_type == Type::Any) &&
                            (right_type == Type::Str || right_type == Type::Any) {
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
                        Ok(Type::Bool)
                    }
                }
            }
            ExprKind::Unary { op, expr: sub_expr } => {
                let expr_type = self.check_expr(sub_expr, None)?;
                match op {
                    UnaryOp::Positive | UnaryOp::Negative => {
                        if expr_type == Type::Int {
                            Ok(Type::Int)
                        } else {
                            Err(format!(
                                "Type Error [{}]: Unary arithmetic operator expects 'int', found '{:?}'", 
                                expr.span, expr_type
                            ))
                        }
                    }
                    UnaryOp::AddressOf => {
                        Ok(Type::Ptr(Box::new(expr_type)))
                    }
                    UnaryOp::Deref => {
                        match expr_type {
                            Type::Ptr(inner_type) => Ok(*inner_type),
                            _ => Err(format!(
                                "Type Error [{}]: Cannot dereference non-pointer type '{:?}'",
                                expr.span, expr_type
                            )),
                        }
                    }
                }
            }
        }
    }

    pub fn check_stmt(&mut self, stmt: &Stmt) -> Result<(), String> {
        match stmt {
            Stmt::Use { .. } => unreachable!(), // Handled by main.rs / lib.rs, you have shit code if this errors.

            Stmt::Extern { name, rttype, params } => {
                let return_type = match rttype {
                    Some(rt) => rt.clone(),
                    None => Type::Void,
                };
                let mut param_types = Vec::new();
                for param in params {
                    let ptype = match &param.ptype {
                        Some(pt) => pt.clone(),
                        None => Type::Any, 
                    };
                    param_types.push(ptype.clone());
                }

                self.declare_function(&name.value, param_types, return_type, name.location.clone())?;
                Ok(())
            }

            Stmt::Assignment { ident, vtype, expr } => {
                let expr_type = self.check_expr(expr, vtype.as_ref())?;

                if let Some(explicit_type) = vtype {
                    if *explicit_type != expr_type {
                        return Err(format!(
                            "Type Error [{}]: Variable '{}' declared as '{:?}' but assigned type '{:?}'",
                            expr.span, ident.value, explicit_type, expr_type
                        ));
                    }
                    self.declare_variable(&ident.value, explicit_type.clone(), ident.location.clone())?;
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

            Stmt::DerefReassignment { target, expr } => {
                let target_resolved_type = self.check_expr(target, None)?;
                let expr_type = self.check_expr(expr, Some(&target_resolved_type))?;

                if target_resolved_type != expr_type {
                    return Err(format!(
                        "Type Error [{}]: Cannot assign type '{:?}' to target location of type '{:?}'",
                        expr.span, expr_type, target_resolved_type
                    ));
                }


                Ok(())
            }

            Stmt::Reassignment { ident, expr } => {
                let expr_type = self.check_expr(expr, None)?;

                let symbol = self.resolve_variable(&ident.value).ok_or_else(|| {
                    format!(
                        "Semantic Error [{}]: Cannot reassign to undefined variable '{}'",
                        ident.location, ident.value
                    )
                })?;

                if symbol.data_type != expr_type {
                    return Err(format!(
                        "Type Error [{}]: Cannot assign type '{:?}' to variable '{}' of type '{:?}'",
                        expr.span, expr_type, ident.value, symbol.data_type
                    ));
                }

                Ok(())
            }
            Stmt::Expr(expr) => {
                self.check_expr(expr, None)?;
                Ok(())
            }

            Stmt::While { cond, body } => {
                let cond_type = self.check_expr(cond, None)?;
                if !self.check_truthiness(&cond_type) {
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

            Stmt::For { init, cond, step, body } => {
                self.enter_scope();

                self.check_stmt(init.as_ref())?;

                let cond_type = self.check_expr(cond, None)?;
                if !self.check_truthiness(&cond_type) {
                    self.leave_scope();
                    return Err(format!(
                        "Type Error [{}]: 'for' condition is not truthy, found '{:?}'",
                        cond.span, cond_type
                    ));
                }

                for block_stmt in body {
                    self.check_stmt(block_stmt)?;
                }

                self.check_stmt(step.as_ref())?;

                self.leave_scope();

                Ok(())
            }

            Stmt::If { cond, then_branch, else_branch } => {
                let cond_type = self.check_expr(cond, None)?;
                if !self.check_truthiness(&cond_type) {
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

                if let Some(else_stmts) = else_branch {
                    self.enter_scope();
                    for block_stmt in else_stmts {
                        self.check_stmt(block_stmt)?;
                    }
                    self.leave_scope();
                }

                Ok(())
            }
            Stmt::Function { name, public: _, rttype, params, body } => {
                let return_type = match rttype {
                    Some(rt) => rt.clone(),
                    None => Type::Void,
                };

                let mut param_types = Vec::new();
                for param in params {
                    let ptype = match &param.ptype {
                        Some(pt) => pt.clone(),
                        None => Type::Any, 
                    };
                    param_types.push(ptype.clone());
                }

                self.declare_function(&name.value, param_types.clone(), return_type.clone(), name.location.clone())?;

                self.enter_scope();
                for (param, ptype) in params.iter().zip(param_types.into_iter()) {
                    self.declare_variable(&param.name.value, ptype, param.name.location.clone())?;
                }

                let prev_return_type = self.current_return_type.replace(return_type.clone());
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
                    Some(e) => self.check_expr(e, None)?,
                    None => Type::Void,
                };

                let expected = self.current_return_type.clone().ok_or_else(|| {
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