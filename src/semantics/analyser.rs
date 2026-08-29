use indexmap::IndexMap;

use crate::parse::parsing::*;
use crate::semantics::analysis::{
    EnumSignature, FunctionSignature, Scope, StructSignature, Symbol,
};
use crate::utils::location::Location;
use crate::utils::typesafe::{self, *};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub enum AnalyserError {
    TypeError { location: Location, message: String },
    SemanticError { location: Location, message: String },
    OverDefinitionError { location: Location, message: String },
}
impl AnalyserError {
    pub fn type_error(location: Location, message: impl Into<String>) -> Self {
        AnalyserError::TypeError {
            location,
            message: message.into(),
        }
    }
    pub fn semantic_error(location: Location, message: impl Into<String>) -> Self {
        AnalyserError::SemanticError {
            location,
            message: message.into(),
        }
    }
    pub fn overdef_error(location: Location, message: impl Into<String>) -> Self {
        AnalyserError::OverDefinitionError {
            location,
            message: message.into(),
        }
    }
}

fn contains_generic_param(ty: &Type) -> bool {
    match ty {
        Type::GenericParam(_) => true,
        Type::Ptr(inner) => contains_generic_param(inner),
        Type::Array { element_type, .. } => contains_generic_param(element_type),
        Type::GenericInstance { args, .. } => args.iter().any(contains_generic_param),
        _ => false,
    }
}

#[derive(Debug)]
pub struct Analyser {
    pub scopes: Vec<Scope>,
    pub current_scope: usize,
    pub functions: HashMap<String, FunctionSignature>,
    pub function_bodies: HashMap<String, (Vec<Parameter>, Vec<Stmt>)>,
    pub structs: HashMap<String, StructSignature>,
    pub enums: HashMap<String, EnumSignature>,
    pub constants: HashMap<String, (Type, Expr)>,
    current_return_type: Option<Type>,
    loop_depth: usize,
    pub current_generic_params: Vec<String>,
}

impl Analyser {
    pub fn new() -> Self {
        Self {
            scopes: vec![Scope {
                symbols: HashMap::new(),
                parent: None,
            }],
            current_scope: 0,
            functions: HashMap::new(),
            function_bodies: HashMap::new(),
            structs: HashMap::new(),
            enums: HashMap::new(),
            constants: HashMap::new(),
            current_return_type: None,
            loop_depth: 0,
            current_generic_params: Vec::new(),
        }
    }

    pub fn enter_scope(&mut self) {
        let parent_idx = self.current_scope;
        let new_scope = Scope {
            symbols: HashMap::new(),
            parent: Some(parent_idx),
        };
        self.scopes.push(new_scope);
        self.current_scope = self.scopes.len() - 1;
    }

    pub fn leave_scope(&mut self) {
        let parent = self.scopes[self.current_scope]
            .parent
            .expect("Attempted to leave global scope");

        self.scopes.pop();
        self.current_scope = parent;
    }

    fn evaluates_statically_to_false(&mut self, cond: &Expr) -> Result<bool, AnalyserError> {
        match &cond.kind {
            ExprKind::Binary { left, op, right } => match op {
                BinaryOp::Eq => {
                    if let (ExprKind::Typeof { expr }, ExprKind::Literal(Literal::String(target))) =
                        (&left.kind, &right.kind)
                    {
                        let actual_type = self.check_expr(expr, None)?;
                        return Ok(type_to_string(&actual_type) != *target);
                    }
                    Ok(false)
                }
                BinaryOp::Or => {
                    let left_false = self.evaluates_statically_to_false(left)?;
                    let right_false = self.evaluates_statically_to_false(right)?;
                    Ok(left_false && right_false)
                }
                BinaryOp::And => {
                    let left_false = self.evaluates_statically_to_false(left)?;
                    let right_false = self.evaluates_statically_to_false(right)?;
                    Ok(left_false || right_false)
                }
                _ => Ok(false),
            },
            _ => Ok(false),
        }
    }

    fn substitute_type(&self, ty: &Type, mapping: &HashMap<String, Type>) -> Type {
        match ty {
            Type::GenericParam(name) => mapping
                .get(name)
                .cloned()
                .unwrap_or_else(|| Type::GenericParam(name.clone())),
            Type::Struct(name) => mapping.get(name).cloned().unwrap_or_else(|| ty.clone()),
            Type::Ptr(inner) => Type::Ptr(Box::new(self.substitute_type(inner, mapping))),
            Type::Array { element_type, size } => Type::Array {
                element_type: Box::new(self.substitute_type(element_type, mapping)),
                size: *size,
            },
            Type::GenericInstance { name, args } => {
                let substituted_args = args
                    .iter()
                    .map(|arg| self.substitute_type(arg, mapping))
                    .collect();
                Type::GenericInstance {
                    name: name.clone(),
                    args: substituted_args,
                }
            }
            _ => ty.clone(),
        }
    }

    fn instantiate_generic_types(
        &mut self,
        ty: &Type,
        span: &Location,
    ) -> Result<Type, AnalyserError> {
        match ty {
            Type::Ptr(inner) => {
                let inst = self.instantiate_generic_types(inner, span)?;
                Ok(Type::Ptr(Box::new(inst)))
            }
            Type::Array { element_type, size } => {
                let inst = self.instantiate_generic_types(element_type, span)?;
                Ok(Type::Array {
                    element_type: Box::new(inst),
                    size: *size,
                })
            }
            Type::GenericInstance { name, args } => {
                let mut resolved_args = Vec::new();
                for arg in args {
                    resolved_args.push(self.instantiate_generic_types(arg, span)?);
                }

                if resolved_args.iter().any(contains_generic_param) {
                    return Ok(Type::GenericInstance {
                        name: name.clone(),
                        args: resolved_args,
                    });
                }

                let template = self
                    .structs
                    .get(name)
                    .ok_or_else(|| AnalyserError::SemanticError {
                        location: span.clone(),
                        message: format!("Semantic Error: Generic struct '{}' not found.", name),
                    })?
                    .clone();

                if template.generic_params.len() != resolved_args.len() {
                    return Err(AnalyserError::TypeError {
                        location: span.clone(),
                        message: format!(
                            "Type Error: Struct '{}' expects {} type parameters, found {}",
                            name,
                            template.generic_params.len(),
                            resolved_args.len()
                        ),
                    });
                }

                let mangled = mangle_name(name, &resolved_args);

                if !self.structs.contains_key(&mangled) {
                    let mapping: HashMap<String, Type> = template
                        .generic_params
                        .iter()
                        .cloned()
                        .zip(resolved_args.iter().cloned())
                        .collect();

                    let mut fresh_fields = IndexMap::new();
                    for (f_name, f_type) in &template.fields {
                        let substituted = self.substitute_type(f_type, &mapping);
                        fresh_fields.insert(f_name.clone(), substituted);
                    }

                    self.structs.insert(
                        mangled.clone(),
                        StructSignature {
                            generic_params: Vec::new(),
                            fields: fresh_fields,
                            location: span.clone(),
                        },
                    );
                }

                Ok(Type::Struct(mangled))
            }
            Type::GenericParam(p) => Ok(Type::GenericParam(p.clone())),
            _ => Ok(ty.clone()),
        }
    }

    fn declare_variable(
        &mut self,
        name: &str,
        data_type: Type,
        span: Location,
    ) -> Result<(), AnalyserError> {
        let scope = &mut self.scopes[self.current_scope];
        if scope.symbols.contains_key(name) {
            return Err(AnalyserError::SemanticError {
                location: span,
                message: format!(
                    "Semantic Error: Variable '{}' already declared in this scope.",
                    name
                ),
            });
        }
        scope.symbols.insert(
            name.to_string(),
            Symbol {
                name: name.to_string(),
                ty: data_type,
            },
        );
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

    fn validate_type_exists(&self, ty: &Type, span: &Location) -> Result<(), AnalyserError> {
        match ty {
            Type::Struct(name) => {
                if self.current_generic_params.contains(name) {
                    return Ok(());
                }

                if !self.structs.contains_key(name) && !self.enums.contains_key(name) {
                    return Err(AnalyserError::SemanticError {
                        location: span.clone(),
                        message: format!(
                            "Semantic Error: Type '{}' is used here but never defined.",
                            name
                        ),
                    });
                }
            }
            Type::GenericInstance { name, args } => {
                if !self.structs.contains_key(name) {
                    return Err(AnalyserError::SemanticError {
                        location: span.clone(),
                        message: format!(
                            "Semantic Error: Generic Struct '{}' is used here but never defined.",
                            name
                        ),
                    });
                }
                for arg in args {
                    self.validate_type_exists(arg, span)?;
                }
            }
            Type::Ptr(inner) => {
                self.validate_type_exists(inner, span)?;
            }
            Type::Array { element_type, .. } => {
                self.validate_type_exists(element_type, span)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn declare_enum(
        &mut self,
        name: &str,
        options: Vec<String>,
        location: Location,
    ) -> Result<(), AnalyserError> {
        if let Some(existing) = self.enums.get(name) {
            return Err(AnalyserError::overdef_error(
                existing.location.clone(),
                format!(
                    "Enum '{}' is already defined at [{}]",
                    name, existing.location
                ),
            ));
        };

        self.enums
            .insert(name.to_string(), EnumSignature { options, location });

        Ok(())
    }

    fn declare_struct(
        &mut self,
        name: &str,
        generic_params: Vec<String>,
        fields: IndexMap<String, Type>,
        location: Location,
    ) -> Result<(), AnalyserError> {
        if let Some(existing) = self.structs.get(name) {
            return Err(AnalyserError::overdef_error(
                location,
                format!(
                    "Struct '{}' is already defined at [{}]",
                    name, existing.location
                ),
            ));
        }

        self.structs.insert(
            name.to_string(),
            StructSignature {
                generic_params,
                fields,
                location,
            },
        );

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn declare_function(
        &mut self,
        name: &str,
        generic_params: Vec<String>,
        public: bool,
        param_types: Vec<Type>,
        is_variadic: bool,
        variadic_name: Option<String>,
        return_type: Type,
        location: Location,
        safe_to_dupe: bool,
    ) -> Result<(), AnalyserError> {
        if let Some(existing) = self.functions.get(name)
            && !safe_to_dupe
        {
            return Err(AnalyserError::SemanticError {
                location,
                message: format!(
                    "Semantic Error: Function '{}' is already defined at [{}]",
                    name, existing.location
                ),
            });
        }

        self.functions.insert(
            name.to_string(),
            FunctionSignature {
                generic_params,
                param_types,
                public,
                is_variadic,
                variadic_param_name: variadic_name,
                return_type,
                location,
            },
        );

        Ok(())
    }

    fn resolve_function(&self, name: &str) -> Option<&FunctionSignature> {
        self.functions.get(name)
    }

    pub fn check_truthiness(&self, ty: &Type) -> bool {
        is_truthy_type(ty)
    }

    pub fn check_expr(
        &mut self,
        expr: &Expr,
        expected_type: Option<&Type>,
    ) -> Result<Type, AnalyserError> {
        match &expr.kind {
            ExprKind::Sizeof { .. } => Ok(Type::Int),
            ExprKind::Typeof { .. } => Ok(Type::Str),
            ExprKind::Cast { left, right } => {
                let leftty = self.check_expr(left.as_ref(), None)?;
                if types_compatible(&leftty, right) {
                    return Ok(right.clone());
                }
                Err(AnalyserError::type_error(
                    expr.span.clone(),
                    format!(
                        "Cannot cast '{}' to '{}'",
                        type_to_string(&leftty),
                        type_to_string(right)
                    ),
                ))
            }
            ExprKind::Literal(lit) => match lit {
                Literal::Int(_) => {
                    if let Some(Type::UInt) = expected_type {
                        Ok(Type::UInt)
                    } else {
                        Ok(Type::Int)
                    }
                }
                Literal::Double(_) => Ok(Type::Double),
                Literal::Float(_) => Ok(Type::Float),
                Literal::String(_) => Ok(Type::Str),
                Literal::Bool(_) => Ok(Type::Bool),
                Literal::Char(_) => Ok(Type::Char),
                Literal::Arr { elements } => {
                    let expected_elem_ty = match expected_type {
                        Some(Type::Array { element_type, .. }) => Some(&**element_type),
                        _ => None,
                    };

                    let element_type = if elements.is_empty() {
                        if let Some(elem_ty) = expected_elem_ty {
                            elem_ty.clone()
                        } else {
                            return Err(AnalyserError::type_error(
                            expr.span.clone(),
                            "Cannot infer the type of an empty array literal without explicit type context."
                                .to_string(),
                        ));
                        }
                    } else {
                        self.check_expr(&elements[0], expected_elem_ty)?
                    };

                    for el in elements {
                        let el_type = self.check_expr(el, Some(&element_type))?;
                        if !types_equal(&element_type, &el_type) {
                            return Err(AnalyserError::type_error(
                                el.span.clone(),
                                format!(
                                    "Heterogeneous array literals are not allowed. Expected elements of type '{}', found '{}'.",
                                    type_to_string(&element_type),
                                    type_to_string(&el_type)
                                ),
                            ));
                        }
                    }

                    Ok(Type::Array {
                        element_type: Box::new(element_type),
                        size: elements.len(),
                    })
                }
            },
            ExprKind::Field { base, field } => {
                if let ExprKind::Identifier(name) = &base.kind
                    && let Some(enum_sig) = self.enums.get(name)
                {
                    if !enum_sig.options.contains(field) {
                        return Err(AnalyserError::semantic_error(
                            expr.span.clone(),
                            format!("Enum '{}' has no variant named '{}'.", name, field),
                        ));
                    }
                    return Ok(Type::Enum(name.clone()));
                }
                let base_type = self.check_expr(base, None)?;

                match base_type {
                    Type::Struct(struct_name) => {
                        let signature = self.structs.get(&struct_name).ok_or_else(|| {
                            AnalyserError::semantic_error(
                                expr.span.clone(),
                                format!(
                                    "Attempted to access field '{}' on undefined struct '{}'.",
                                    field, struct_name
                                ),
                            )
                        })?;

                        let field_type = signature.fields.get(field).ok_or_else(|| {
                            AnalyserError::semantic_error(
                                expr.span.clone(),
                                format!("Struct '{}' has no field named '{}'.", struct_name, field),
                            )
                        })?;

                        Ok(field_type.clone())
                    }
                    Type::GenericInstance { name, args } => {
                        let signature = self.structs.get(&name).ok_or_else(|| {
                            AnalyserError::semantic_error(
                                expr.span.clone(),
                                format!(
                                    "Attempted to access field '{}' on undefined struct '{}'.",
                                    field, name
                                ),
                            )
                        })?;

                        let raw_field_type = signature.fields.get(field).ok_or_else(|| {
                            AnalyserError::semantic_error(
                                expr.span.clone(),
                                format!("Struct '{}' has no field named '{}'.", name, field),
                            )
                        })?;

                        let mapping: HashMap<String, Type> = signature
                            .generic_params
                            .iter()
                            .cloned()
                            .zip(args.iter().cloned())
                            .collect();

                        Ok(self.substitute_type(raw_field_type, &mapping))
                    }
                    Type::VariadicPack { name: _, types } => {
                        if types.is_empty() {
                            match variadic::parse_field(field) {
                                Some(variadic::PackField::Index(_)) => Ok(Type::Any),
                                Some(variadic::PackField::Length) => Ok(Type::Int),
                                None => Err(AnalyserError::semantic_error(
                                    expr.span.clone(),
                                    format!("Invalid variadic field '{}'.", field),
                                )),
                            }
                        } else {
                            let signature = variadic::structure(&types, expr.span.clone());

                            let field_type = signature.fields.get(field).ok_or_else(|| {
                                AnalyserError::semantic_error(
                                    expr.span.clone(),
                                    format!("Invalid variadic field '{}'.", field),
                                )
                            })?;

                            Ok(field_type.clone())
                        }
                    }
                    Type::Any => Ok(Type::Any),
                    _ => Err(AnalyserError::type_error(
                        expr.span.clone(),
                        format!(
                            "Cannot access a field on non-struct type '{}'.",
                            type_to_string(&base_type)
                        ),
                    )),
                }
            }
            ExprKind::StructLiteral {
                struct_name,
                generic_args,
                fields,
            } => {
                let concrete_ty = if generic_args.is_empty() {
                    let template = self.structs.get(struct_name).ok_or_else(|| {
                        AnalyserError::semantic_error(
                            expr.span.clone(),
                            format!("Undefined struct '{}'.", struct_name),
                        )
                    })?;
                    if !template.generic_params.is_empty() {
                        return Err(AnalyserError::type_error(
                            expr.span.clone(),
                            format!("Struct '{}' requires generic arguments.", struct_name),
                        ));
                    }
                    Type::Struct(struct_name.clone())
                } else {
                    let generic_ty = Type::GenericInstance {
                        name: struct_name.clone(),
                        args: generic_args.clone(),
                    };
                    self.validate_type_exists(&generic_ty, &expr.span)?;
                    self.instantiate_generic_types(&generic_ty, &expr.span)?
                };

                let concrete_name = match &concrete_ty {
                    Type::Struct(name) => name.clone(),
                    _ => {
                        return Err(AnalyserError::type_error(
                            expr.span.clone(),
                            format!("Expected concrete struct type, got {:?}", concrete_ty),
                        ));
                    }
                };
                let struct_def = self
                    .structs
                    .get(&concrete_name)
                    .ok_or_else(|| {
                        AnalyserError::semantic_error(
                            expr.span.clone(),
                            format!("Instantiated struct '{}' not found.", concrete_name),
                        )
                    })?
                    .clone();

                if fields.len() != struct_def.fields.len() {
                    return Err(AnalyserError::type_error(
                        expr.span.clone(),
                        format!(
                            "Struct '{}' expects {} fields, found {}.",
                            concrete_name,
                            struct_def.fields.len(),
                            fields.len()
                        ),
                    ));
                }

                let mut seen_fields = HashSet::new();
                for (field_name, field_expr) in fields {
                    if !seen_fields.insert(field_name) {
                        return Err(AnalyserError::semantic_error(
                            field_expr.span.clone(),
                            format!("Duplicate field '{}' in struct literal.", field_name),
                        ));
                    }

                    let expected_ty = struct_def.fields.get(field_name).ok_or_else(|| {
                        AnalyserError::semantic_error(
                            field_expr.span.clone(),
                            format!(
                                "Field '{}' does not exist in struct '{}'.",
                                field_name, concrete_name
                            ),
                        )
                    })?;

                    let actual_ty = self.check_expr(field_expr, Some(expected_ty))?;
                    if !types_equal(expected_ty, &actual_ty) {
                        return Err(AnalyserError::type_error(
                            field_expr.span.clone(),
                            format!(
                                "Field '{}' expects type '{}', but found '{}'.",
                                field_name,
                                type_to_string(expected_ty),
                                type_to_string(&actual_ty)
                            ),
                        ));
                    }
                }

                Ok(concrete_ty)
            }
            ExprKind::Index { base, index } => {
                let base_type = self.check_expr(base, None)?;
                let index_type = self.check_expr(index, Some(&Type::Int))?;

                if !is_integer(&index_type) {
                    return Err(AnalyserError::type_error(
                        index.span.clone(),
                        format!(
                            "Array index must be an integer, found '{}'.",
                            type_to_string(&index_type)
                        ),
                    ));
                }

                match base_type {
                    Type::Array { element_type, .. } => Ok(*element_type),
                    Type::Str => Ok(Type::Char),
                    Type::Ptr(inner_type) => Ok(*inner_type),
                    _ => Err(AnalyserError::type_error(
                        expr.span.clone(),
                        format!(
                            "Cannot index into non-indexable type '{}'.",
                            type_to_string(&base_type)
                        ),
                    )),
                }
            }
            ExprKind::Identifier(name) => {
                if let Some(symbol) = self.resolve_variable(name) {
                    Ok(symbol.ty.clone())
                } else if let Some((const_type, _)) = self.constants.get(name) {
                    Ok(const_type.clone())
                } else {
                    Err(AnalyserError::semantic_error(
                        expr.span.clone(),
                        format!("Symbol '{}' is used before definition.", name),
                    ))
                }
            }
            ExprKind::Call {
                callee,
                generic_args,
                args,
            } => {
                let template = self
                    .resolve_function(&callee.value)
                    .ok_or_else(|| {
                        AnalyserError::semantic_error(
                            callee.location.clone(),
                            format!("Call to undefined function '{}'", callee.value),
                        )
                    })?
                    .clone();

                let mut resolved_func_name = callee.value.clone();

                // Resolve generic function arguments and create the concrete signature.
                if !template.generic_params.is_empty() || !generic_args.is_empty() {
                    if template.generic_params.len() != generic_args.len() {
                        return Err(AnalyserError::type_error(
                            expr.span.clone(),
                            format!(
                                "Function '{}' expects {} type parameters, found {}",
                                callee.value,
                                template.generic_params.len(),
                                generic_args.len()
                            ),
                        ));
                    }

                    let mut inst_args = Vec::new();

                    for g_arg in generic_args {
                        inst_args.push(self.instantiate_generic_types(g_arg, &callee.location)?);
                    }

                    resolved_func_name = mangle_name(&callee.value, &inst_args);

                    if !self.functions.contains_key(&resolved_func_name) {
                        let mut mapping = HashMap::new();

                        for (param_name, concrete_type) in
                            template.generic_params.iter().zip(&inst_args)
                        {
                            mapping.insert(param_name.clone(), concrete_type.clone());
                            mapping
                                .insert(format!("gparam__{}", param_name), concrete_type.clone());
                        }

                        let substituted_return =
                            self.substitute_type(&template.return_type, &mapping);

                        let fresh_return =
                            self.instantiate_generic_types(&substituted_return, &callee.location)?;

                        let mut fresh_params = Vec::new();

                        for p_ty in &template.param_types {
                            let substituted_param = self.substitute_type(p_ty, &mapping);

                            let fully_resolved_param = self
                                .instantiate_generic_types(&substituted_param, &callee.location)?;

                            fresh_params.push(fully_resolved_param);
                        }

                        self.functions.insert(
                            resolved_func_name.clone(),
                            FunctionSignature {
                                generic_params: Vec::new(),
                                param_types: fresh_params,
                                public: template.public,
                                is_variadic: template.is_variadic,
                                variadic_param_name: template.variadic_param_name.clone(),
                                return_type: fresh_return,
                                location: template.location.clone(),
                            },
                        );
                    }
                }

                let sig = self.functions.get(&resolved_func_name).unwrap().clone();

                if !sig.public && sig.location.file != callee.location.file {
                    return Err(AnalyserError::semantic_error(
                        callee.location.clone(),
                        format!(
                            "Function '{}' is private and cannot be called from another module",
                            callee.value
                        ),
                    ));
                }

                // `param_types` contains ONLY fixed parameters.
                let fixed_arg_count = sig.param_types.len();

                if sig.is_variadic {
                    if args.len() < fixed_arg_count {
                        return Err(AnalyserError::type_error(
                            expr.span.clone(),
                            format!(
                                "Function '{}' expects at least {} argument(s), found {}",
                                callee.value,
                                fixed_arg_count,
                                args.len()
                            ),
                        ));
                    }
                } else if args.len() != fixed_arg_count {
                    return Err(AnalyserError::type_error(
                        expr.span.clone(),
                        format!(
                            "Function '{}' expects {} argument(s), found {}",
                            callee.value,
                            fixed_arg_count,
                            args.len()
                        ),
                    ));
                }

                let param_types = sig.param_types.clone();
                let return_type = sig.return_type.clone();

                // Check fixed arguments.
                for (i, (arg, expected)) in args[..fixed_arg_count]
                    .iter()
                    .zip(param_types.iter())
                    .enumerate()
                {
                    let arg_type = self.check_expr(arg, Some(expected))?;

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
                            if **expected_elem != Type::Any
                                && !types_equal(expected_elem, actual_elem)
                            {
                                return Err(AnalyserError::type_error(
                                    arg.span.clone(),
                                    format!(
                                        "Argument {} to '{}' expects array of '{}', found array of '{}'",
                                        i + 1,
                                        callee.value,
                                        type_to_string(expected_elem),
                                        type_to_string(actual_elem),
                                    ),
                                ));
                            }
                        }

                        (Type::Array { .. }, _) => {
                            return Err(AnalyserError::type_error(
                                arg.span.clone(),
                                format!(
                                    "Argument {} to '{}' expects '{}', found '{}'",
                                    i + 1,
                                    callee.value,
                                    type_to_string(expected),
                                    type_to_string(&arg_type),
                                ),
                            ));
                        }

                        _ => {
                            if !types_equal(expected, &arg_type) {
                                return Err(AnalyserError::type_error(
                                    arg.span.clone(),
                                    format!(
                                        "Argument {} to '{}' expects '{}', found '{}'",
                                        i + 1,
                                        callee.value,
                                        type_to_string(expected),
                                        type_to_string(&arg_type),
                                    ),
                                ));
                            }
                        }
                    }
                }

                if sig.is_variadic {
                    let variadic_args = &args[fixed_arg_count..];

                    let mut variadic_types = Vec::new();

                    for arg in variadic_args {
                        variadic_types.push(self.check_expr(arg, None)?);
                    }

                    let variadic_mangled_name =
                        mangle_variadic(&resolved_func_name, &variadic_types);

                    if !self.functions.contains_key(&variadic_mangled_name) {
                        self.functions.insert(
                            variadic_mangled_name.clone(),
                            FunctionSignature {
                                generic_params: Vec::new(),
                                param_types: param_types.clone(),

                                public: sig.public,

                                is_variadic: true,
                                variadic_param_name: sig.variadic_param_name.clone(),
                                return_type: return_type.clone(),
                                location: sig.location.clone(),
                            },
                        );

                        if let Some(variadic_name) = &sig.variadic_param_name
                            && let Some((params, func_body)) =
                                self.function_bodies.get(&callee.value).cloned()
                        {
                            self.enter_scope();

                            let mut fixed_index = 0;

                            for param in &params {
                                if param.is_variadic {
                                    continue;
                                }

                                let param_type =
                                    param_types.get(fixed_index).cloned().ok_or_else(|| {
                                        AnalyserError::semantic_error(
                                            param.name.location.clone(),
                                            format!(
                                                "Internal error: missing type for parameter '{}'",
                                                param.name.value
                                            ),
                                        )
                                    })?;

                                self.declare_variable(
                                    &param.name.value,
                                    param_type,
                                    param.name.location.clone(),
                                )?;

                                fixed_index += 1;
                            }
                            self.declare_variable(
                                variadic_name,
                                Type::VariadicPack {
                                    name: variadic_name.clone(),
                                    types: variadic_types,
                                },
                                callee.location.clone(),
                            )?;

                            for stmt in &func_body {
                                self.check_stmt(stmt, TypeCheckMode::Strict)?;
                            }

                            self.leave_scope();
                        }
                    }
                }

                Ok(return_type)
            }

            ExprKind::Binary { left, op, right } => {
                let left_type = self.check_expr(left, None)?;
                let right_type = self.check_expr(right, Some(&left_type))?;

                match op {
                    BinaryOp::Add => {
                        if left_type == Type::Any || right_type == Type::Any {
                            return Ok(Type::Any);
                        }
                        if is_integer(&left_type) && is_integer(&right_type) {
                            if left_type == right_type {
                                Ok(left_type)
                            } else {
                                Err(AnalyserError::type_error(
                                    expr.span.clone(),
                                    format!(
                                        "Cannot add mismatched integer types '{}' and '{}'",
                                        type_to_string(&left_type),
                                        type_to_string(&right_type)
                                    ),
                                ))
                            }
                        } else if Type::Str != left_type && Type::Str != right_type {
                            Ok(Type::Str)
                        } else {
                            Err(AnalyserError::type_error(
                                expr.span.clone(),
                                format!(
                                    "Cannot add type '{}' and '{}'",
                                    type_to_string(&left_type),
                                    type_to_string(&right_type)
                                ),
                            ))
                        }
                    }
                    BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
                        if is_integer(&left_type) && is_integer(&right_type) {
                            if types_equal(&left_type, &right_type) {
                                Ok(left_type)
                            } else {
                                Err(AnalyserError::type_error(
                                    expr.span.clone(),
                                    format!(
                                        "Mixed-type integer arithmetic ('{}' and '{}') is not allowed",
                                        type_to_string(&left_type),
                                        type_to_string(&right_type)
                                    ),
                                ))
                            }
                        } else {
                            Err(AnalyserError::type_error(
                                expr.span.clone(),
                                format!(
                                    "Operator '{:?}' expects integers, but found '{}' and '{}'",
                                    op,
                                    type_to_string(&left_type),
                                    type_to_string(&right_type)
                                ),
                            ))
                        }
                    }
                    BinaryOp::Eq
                    | BinaryOp::NEq
                    | BinaryOp::Gt
                    | BinaryOp::GtE
                    | BinaryOp::And
                    | BinaryOp::Or
                    | BinaryOp::Lt
                    | BinaryOp::LtE => {
                        if types_equal(&left_type, &right_type) {
                            Ok(Type::Bool)
                        } else {
                            Err(AnalyserError::type_error(
                                expr.span.clone(),
                                format!(
                                    "Cannot compare incompatible types '{}' and '{}'",
                                    type_to_string(&left_type),
                                    type_to_string(&right_type)
                                ),
                            ))
                        }
                    }
                }
            }
            ExprKind::Unary { op, expr: sub_expr } => {
                let expr_type = self.check_expr(sub_expr, None)?;
                match op {
                    UnaryOp::Positive | UnaryOp::Negative => {
                        if is_signed_integer(&expr_type) {
                            Ok(expr_type)
                        } else {
                            Err(AnalyserError::type_error(
                                expr.span.clone(),
                                format!(
                                    "Unary sign operators are only supported on signed integers, found '{}'",
                                    type_to_string(&expr_type)
                                ),
                            ))
                        }
                    }
                    UnaryOp::Not => {
                        if expr_type == Type::Bool {
                            Ok(Type::Bool)
                        } else {
                            Err(AnalyserError::type_error(
                                expr.span.clone(),
                                format!(
                                    "Unary boolean operator expects 'bool', found '{}'",
                                    type_to_string(&expr_type)
                                ),
                            ))
                        }
                    }
                    UnaryOp::AddressOf => Ok(Type::Ptr(Box::new(expr_type))),
                    UnaryOp::Deref => match expr_type {
                        Type::Ptr(inner_type) => Ok(*inner_type),
                        _ => Err(AnalyserError::type_error(
                            expr.span.clone(),
                            format!(
                                "Cannot dereference non-pointer type '{}'",
                                type_to_string(&expr_type)
                            ),
                        )),
                    },
                }
            }
        }
    }

    pub fn check_stmt(&mut self, stmt: &Stmt, mode: TypeCheckMode) -> Result<(), AnalyserError> {
        match stmt {
            Stmt::Use { .. } => unreachable!(),

            Stmt::Enum { name, options } => {
                self.declare_enum(
                    &name.value,
                    options.iter().map(|ident| ident.value.clone()).collect(),
                    name.location.clone(),
                )?;
                Ok(())
            }

            Stmt::Struct {
                name,
                generic_params,
                fields,
            } => {
                let mut struct_fields = IndexMap::new();

                let prev_generic_params =
                    std::mem::replace(&mut self.current_generic_params, generic_params.clone());

                for field in fields {
                    let field_type = match &field.ptype {
                        Some(t) => t.clone(),
                        None => Type::Any,
                    };

                    if struct_fields.contains_key(&field.name.value) {
                        return Err(AnalyserError::SemanticError {
                            location: field.name.location.clone(),
                            message: format!(
                                "Semantic Error: Struct '{}' contains duplicate field '{}'",
                                name.value, field.name.value
                            ),
                        });
                    }

                    struct_fields.insert(field.name.value.clone(), field_type);
                }

                self.declare_struct(
                    &name.value,
                    generic_params.clone(),
                    struct_fields,
                    name.location.clone(),
                )?;

                self.current_generic_params = prev_generic_params;

                Ok(())
            }

            Stmt::Break { location } => {
                if self.loop_depth == 0 {
                    return Err(AnalyserError::SemanticError {
                        location: location.clone(),
                        message:
                            "Semantic Error: Break statement must be in a while loop statement"
                                .to_string(),
                    });
                }
                Ok(())
            }

            Stmt::Extern {
                name,
                rttype,
                generic_params,
                params,
            } => {
                let return_type = match rttype {
                    Some(rt) => {
                        let instantiated = self.instantiate_generic_types(rt, &name.location)?;
                        self.validate_type_exists(&instantiated, &name.location)?;
                        instantiated
                    }
                    None => Type::Void,
                };

                let prev_generic_params =
                    std::mem::replace(&mut self.current_generic_params, generic_params.clone());

                let mut param_types = Vec::new();
                let mut is_variadic = false;
                let mut variadic_param = None;
                for param in params {
                    if param.is_variadic {
                        is_variadic = true;
                        variadic_param = Some(param.name.value.clone());
                        continue;
                    }
                    let ptype = match &param.ptype {
                        Some(pt) => {
                            let instantiated =
                                self.instantiate_generic_types(pt, &param.name.location)?;
                            self.validate_type_exists(&instantiated, &param.name.location)?;
                            instantiated
                        }
                        None => Type::Any,
                    };
                    param_types.push(ptype);
                }

                self.current_generic_params = prev_generic_params;

                self.declare_function(
                    &name.value,
                    generic_params.clone(),
                    true,
                    param_types,
                    is_variadic,
                    variadic_param,
                    return_type,
                    name.location.clone(),
                    true,
                )?;
                Ok(())
            }

            Stmt::Constant { name, vtype, expr } => {
                if self.current_return_type.is_some() {
                    return Err(AnalyserError::SemanticError {
                        location: name.location.clone(),
                        message: format!(
                            "Semantic Error: Constant '{}' cannot be defined inside a function.",
                            name.value
                        ),
                    });
                }

                let const_type = match (vtype, expr) {
                    (Some(explicit_type), expr_node) => {
                        let instantiated =
                            self.instantiate_generic_types(explicit_type, &name.location)?;
                        self.validate_type_exists(&instantiated, &name.location)?;
                        let expr_type = self.check_expr(expr_node, Some(&instantiated))?;
                        if !types_equal(&instantiated, &expr_type) {
                            return Err(AnalyserError::TypeError {
                                location: expr_node.span.clone(),
                                message: format!(
                                    "Type Error: Constant '{}' declared as '{}' but initialiser has type '{}'",
                                    name.value,
                                    type_to_string(&instantiated),
                                    type_to_string(&expr_type)
                                ),
                            });
                        }
                        instantiated
                    }
                    (None, expr_node) => self.check_expr(expr_node, None)?,
                };

                if self.constants.contains_key(&name.value) {
                    return Err(AnalyserError::SemanticError {
                        location: name.location.clone(),
                        message: format!(
                            "Semantic Error: Constant '{}' already defined.",
                            name.value
                        ),
                    });
                }

                self.constants
                    .insert(name.value.clone(), (const_type, expr.clone()));

                Ok(())
            }

            Stmt::Assignment { ident, vtype, expr } => {
                let variable_type = match (vtype, expr) {
                    (Some(explicit_type), Some(expr_node)) => {
                        let instantiated =
                            self.instantiate_generic_types(explicit_type, &ident.location)?;
                        self.validate_type_exists(&instantiated, &ident.location)?;
                        let expr_type = self.check_expr(expr_node, Some(&instantiated))?;

                        if !types_equal(&instantiated, &expr_type) {
                            return Err(AnalyserError::type_error(
                                expr_node.span.clone(),
                                format!(
                                    "Variable '{}' declared as '{}' but assigned type '{}'",
                                    ident.value,
                                    type_to_string(&instantiated),
                                    type_to_string(&expr_type)
                                ),
                            ));
                        }
                        instantiated
                    }
                    (Some(explicit_type), None) => {
                        let instantiated =
                            self.instantiate_generic_types(explicit_type, &ident.location)?;
                        self.validate_type_exists(&instantiated, &ident.location)?;
                        instantiated
                    }
                    (None, Some(expr_node)) => self.check_expr(expr_node, None)?,
                    (None, None) => {
                        return Err(AnalyserError::semantic_error(
                            ident.location.clone(),
                            format!(
                                "Variable '{}' declared without an explicit type or initializer expression.",
                                ident.value
                            ),
                        ));
                    }
                };

                if let Some(existing_symbol) = self.resolve_variable(&ident.value) {
                    if !types_equal(&existing_symbol.ty, &variable_type) {
                        return Err(AnalyserError::type_error(
                            ident.location.clone(),
                            format!(
                                "Cannot reassign type '{}' to variable '{}' of type '{}'",
                                type_to_string(&variable_type),
                                ident.value,
                                type_to_string(&existing_symbol.ty)
                            ),
                        ));
                    }
                } else {
                    self.declare_variable(&ident.value, variable_type, ident.location.clone())?;
                }

                Ok(())
            }

            Stmt::DerefReassignment { target, expr } => {
                let target_resolved_type = self.check_expr(target, None)?;
                let expr_type = self.check_expr(expr, Some(&target_resolved_type))?;

                if !types_equal(&target_resolved_type, &expr_type) {
                    return Err(AnalyserError::type_error(
                        expr.span.clone(),
                        format!(
                            "Cannot assign type '{}' to target location of type '{}'",
                            type_to_string(&expr_type),
                            type_to_string(&target_resolved_type)
                        ),
                    ));
                }

                Ok(())
            }

            Stmt::Reassignment { ident, expr } => {
                let expected_ty = self
                    .resolve_variable(&ident.value)
                    .map(|symbol| symbol.ty.clone())
                    .ok_or_else(|| {
                        AnalyserError::semantic_error(
                            ident.location.clone(),
                            format!("Cannot reassign to undefined variable '{}'", ident.value),
                        )
                    })?;

                let expr_type = self.check_expr(expr, Some(&expected_ty))?;

                if !types_equal(&expected_ty, &expr_type) {
                    return Err(AnalyserError::type_error(
                        expr.span.clone(),
                        format!(
                            "Cannot assign type '{}' to variable '{}' of type '{}'",
                            type_to_string(&expr_type),
                            ident.value,
                            type_to_string(&expected_ty)
                        ),
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
                    return Err(AnalyserError::type_error(
                        cond.span.clone(),
                        format!(
                            "'while' condition is not truthy, found '{}'",
                            type_to_string(&cond_type)
                        ),
                    ));
                }
                self.enter_scope();
                self.loop_depth += 1;
                for block_stmt in body {
                    self.check_stmt(block_stmt, mode)?;
                }
                self.leave_scope();
                self.loop_depth -= 1;
                Ok(())
            }

            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                self.enter_scope();

                self.check_stmt(init.as_ref(), mode)?;

                let cond_type = self.check_expr(cond, None)?;
                if !self.check_truthiness(&cond_type) {
                    self.leave_scope();
                    return Err(AnalyserError::type_error(
                        cond.span.clone(),
                        format!(
                            "'for' condition is not truthy, found '{}'",
                            type_to_string(&cond_type)
                        ),
                    ));
                }

                for block_stmt in body {
                    self.check_stmt(block_stmt, mode)?;
                }

                self.check_stmt(step.as_ref(), mode)?;

                self.leave_scope();

                Ok(())
            }
            Stmt::ForIn {
                field_ident,
                target_expr,
                body,
            } => {
                let target_type = self.check_expr(target_expr, None)?;

                if let Type::VariadicPack { types, .. } = &target_type {
                    let iter_types = if types.is_empty() {
                        vec![Type::Any]
                    } else {
                        types.clone()
                    };

                    for elem_type in iter_types {
                        self.enter_scope();

                        self.declare_variable(
                            &field_ident.value,
                            elem_type.clone(),
                            field_ident.location.clone(),
                        )?;

                        self.loop_depth += 1;

                        for body_stmt in body {
                            self.check_stmt(body_stmt, mode)?;
                        }

                        self.loop_depth -= 1;
                        self.leave_scope();
                    }

                    return Ok(());
                }

                let elements = typesafe::iterable_elements(&target_type, &self.structs)
                    .ok_or_else(|| {
                        AnalyserError::type_error(
                            target_expr.span.clone(),
                            format!("Type '{}' is not iterable.", type_to_string(&target_type)),
                        )
                    })?;

                self.enter_scope();

                let element_type = elements.first().map(|(_, ty)| ty.clone()).ok_or_else(|| {
                    AnalyserError::type_error(
                        target_expr.span.clone(),
                        "Cannot infer the element type of an empty iterable.".to_string(),
                    )
                })?;

                self.declare_variable(
                    &field_ident.value,
                    element_type,
                    field_ident.location.clone(),
                )?;

                self.loop_depth += 1;

                for body_stmt in body {
                    self.check_stmt(body_stmt, mode)?;
                }

                self.loop_depth -= 1;
                self.leave_scope();

                Ok(())
            }
            Stmt::If {
                cond,
                then_branch,
                else_if_branches,
                else_branch,
            } => {
                let cond_type = self.check_expr(cond, None)?;
                if !self.check_truthiness(&cond_type) {
                    return Err(AnalyserError::type_error(
                        cond.span.clone(),
                        format!(
                            "'if' condition is not truthy, found '{}'",
                            type_to_string(&cond_type)
                        ),
                    ));
                }

                if !self.evaluates_statically_to_false(cond)? {
                    self.enter_scope();
                    for block_stmt in then_branch {
                        self.check_stmt(block_stmt, mode)?;
                    }
                    self.leave_scope();
                }

                for (cond, body) in else_if_branches {
                    let cond_type = self.check_expr(cond, None)?;
                    if !self.check_truthiness(&cond_type) {
                        return Err(AnalyserError::type_error(
                            cond.span.clone(),
                            format!(
                                "'elseif' condition is not truthy, found '{}'",
                                type_to_string(&cond_type)
                            ),
                        ));
                    }

                    if !self.evaluates_statically_to_false(cond)? {
                        self.enter_scope();
                        for body_stmt in body {
                            self.check_stmt(body_stmt, mode)?;
                        }
                        self.leave_scope();
                    }
                }

                if let Some(else_stmts) = else_branch {
                    self.enter_scope();
                    for block_stmt in else_stmts {
                        self.check_stmt(block_stmt, mode)?;
                    }
                    self.leave_scope();
                }

                Ok(())
            }
            Stmt::Function {
                name,
                public,
                rttype,
                generic_params,
                params,
                body,
            } => {
                let is_generic = !generic_params.is_empty();

                let return_type = match rttype {
                    Some(rt) => {
                        if is_generic {
                            rt.clone()
                        } else {
                            let instantiated =
                                self.instantiate_generic_types(rt, &name.location)?;
                            self.validate_type_exists(&instantiated, &name.location)?;
                            instantiated
                        }
                    }
                    None => Type::Void,
                };

                let prev_generic_params =
                    std::mem::replace(&mut self.current_generic_params, generic_params.clone());

                let mut param_types = Vec::new();
                let mut is_variadic = false;
                let mut variadic_param = None;
                for param in params {
                    if param.is_variadic {
                        is_variadic = true;
                        variadic_param = Some(param.name.value.clone());
                        continue;
                    }

                    let ptype = match &param.ptype {
                        Some(pt) => {
                            if is_generic {
                                pt.clone()
                            } else {
                                let instantiated =
                                    self.instantiate_generic_types(pt, &param.name.location)?;
                                self.validate_type_exists(&instantiated, &param.name.location)?;
                                instantiated
                            }
                        }
                        None => Type::Any,
                    };

                    param_types.push(ptype);
                }

                self.function_bodies
                    .insert(name.value.clone(), (params.clone(), body.clone()));
                self.declare_function(
                    &name.value,
                    generic_params.clone(),
                    *public,
                    param_types.clone(),
                    is_variadic,
                    variadic_param,
                    return_type.clone(),
                    name.location.clone(),
                    false,
                )?;

                self.enter_scope();

                for (param, ptype) in params.iter().filter(|p| !p.is_variadic).zip(param_types) {
                    self.declare_variable(&param.name.value, ptype, param.name.location.clone())?;
                }

                if let Some(variadic_param) = params.iter().find(|p| p.is_variadic) {
                    self.declare_variable(
                        &variadic_param.name.value,
                        Type::VariadicPack {
                            name: variadic_param.name.value.clone(),
                            types: Vec::new(),
                        },
                        variadic_param.name.location.clone(),
                    )?;
                }

                let prev_return_type = self.current_return_type.replace(return_type.clone());

                let body_mode = if is_variadic {
                    TypeCheckMode::Passive
                } else {
                    mode
                };

                let mut returns = false;

                for block_stmt in body {
                    if let Stmt::Return { .. } = block_stmt {
                        returns = true;
                    }

                    self.check_stmt(block_stmt, body_mode)?;
                }

                self.current_return_type = prev_return_type;

                if return_type != Type::Void && !returns {
                    return Err(AnalyserError::TypeError {
                        location: name.location.clone(),
                        message: format!(
                            "Type Error: Function '{}' must return a value of type '{}'",
                            name.value,
                            type_to_string(&return_type)
                        ),
                    });
                }

                self.current_generic_params = prev_generic_params;
                self.leave_scope();

                Ok(())
            }

            Stmt::Return { value, span } => {
                let expected = self.current_return_type.clone().ok_or_else(|| {
                    AnalyserError::SemanticError {
                        location: span.clone(),
                        message: "Semantic Error: 'return' used outside of a function".to_string(),
                    }
                })?;

                let actual_type = match value {
                    Some(e) => self.check_expr(e, Some(&expected))?,
                    None => Type::Void,
                };

                if !types_equal(&expected, &actual_type) {
                    return Err(AnalyserError::TypeError {
                        location: span.clone(),
                        message: format!(
                            "Function expects return type '{}', found '{}'",
                            type_to_string(&expected),
                            type_to_string(&actual_type)
                        ),
                    });
                }

                Ok(())
            }
        }
    }

    pub fn analyse(&mut self, program: &Program) -> Result<(), AnalyserError> {
        for stmt in &program.statements {
            self.check_stmt(stmt, TypeCheckMode::Strict)?;
        }
        Ok(())
    }
}
impl Default for Analyser {
    fn default() -> Self {
        Self::new()
    }
}
