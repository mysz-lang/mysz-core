use std::collections::HashMap;

use indexmap::IndexMap;

use crate::{
    ir::tac::{CastType, Instruction, IrOp, ScopedMap, Value},
    parse::parsing::{BinaryOp, Expr, ExprKind, Literal, Parameter, Program, Stmt, Type, UnaryOp},
    utils::location::Location,
    utils::typesafe::type_to_string,
};

use crate::utils::typesafe;
use crate::utils::typesafe::variadic;

#[derive(Debug, Clone, PartialEq)]
pub enum ConstVal {
    Bool(bool),
    Str(String),
    Char(char),
    Int(i64),
}

pub struct TempGen {
    counter: usize,
}
impl TempGen {
    pub fn new() -> Self {
        Self { counter: 0 }
    }
    pub fn next_temp(&mut self) -> String {
        self.counter += 1;
        format!("t{}", self.counter)
    }
}
impl Default for TempGen {
    fn default() -> Self {
        Self::new()
    }
}

pub struct LabelGen {
    counter: usize,
}
impl LabelGen {
    pub fn new() -> Self {
        Self { counter: 0 }
    }
    pub fn next_label(&mut self) -> String {
        self.counter += 1;
        format!("L{}", self.counter)
    }
}
impl Default for LabelGen {
    fn default() -> Self {
        Self::new()
    }
}
pub struct FunctionGen {
    counter: usize,
}
impl FunctionGen {
    pub fn new() -> Self {
        Self { counter: 0 }
    }
    pub fn next(&mut self, name: String) -> String {
        self.counter += 1;
        name
    }
}
impl Default for FunctionGen {
    fn default() -> Self {
        Self::new()
    }
}
#[derive(Debug, Clone)]
pub struct StructLayout {
    pub total_size: i64,
    pub alignment: i64,
    pub field_offsets: IndexMap<String, (i64, Type)>,
}

pub struct IRGen {
    pub code: Vec<Instruction>,
    temps: TempGen,
    labels: LabelGen,
    functions: FunctionGen,
    loop_exits: Vec<String>,
    pub analyser_constants: HashMap<String, (Type, Expr)>,
    pub evaluated_constants: HashMap<String, Value>,
    pub var_types: ScopedMap,
    pub struct_defs: HashMap<String, StructLayout>,
    pub struct_blueprints: HashMap<String, (Vec<String>, Vec<Parameter>)>,
    pub enum_defs: HashMap<String, IndexMap<String, i64>>,
    pub current_function: String,

    pub fn_blueprints: HashMap<String, Stmt>,
    pub instantiated_fns: std::collections::HashSet<String>,
    pub deferred_instantiations: Vec<(String, Vec<Type>, Vec<Type>)>, // (callee_name, generic_args, variadic_arg_types)
    pub current_substitutions: HashMap<String, Type>,

    pub var_aliases: Vec<HashMap<String, String>>,
}

impl IRGen {
    pub fn new() -> Self {
        Self {
            code: Vec::new(),
            temps: TempGen::new(),
            labels: LabelGen::new(),
            functions: FunctionGen::new(),
            loop_exits: Vec::new(),
            struct_defs: HashMap::new(),
            struct_blueprints: HashMap::new(),
            enum_defs: HashMap::new(),
            var_types: ScopedMap::new(HashMap::new()),
            current_function: String::new(),

            analyser_constants: HashMap::new(),
            evaluated_constants: HashMap::new(),

            fn_blueprints: HashMap::new(),
            instantiated_fns: std::collections::HashSet::new(),
            deferred_instantiations: Vec::new(),
            current_substitutions: HashMap::new(),

            var_aliases: vec![HashMap::new()],
        }
    }
    pub fn eval_const(&mut self, expr: &Expr) -> Option<ConstVal> {
        match &expr.kind {
            ExprKind::Literal(lit) => match lit {
                Literal::Int(i) => Some(ConstVal::Int(*i)),
                Literal::String(s) => Some(ConstVal::Str(s.clone())),
                Literal::Char(c) => Some(ConstVal::Char(*c)),
                Literal::Bool(b) => Some(ConstVal::Bool(*b)),
                _ => None,
            },

            ExprKind::Typeof { expr: inner } => {
                let ty = self.type_of_expr(inner)?;
                Some(ConstVal::Str(typesafe::typeof_string(&ty)))
            }

            ExprKind::Binary { left, op, right } => {
                let l = self.eval_const(left)?;
                let r = self.eval_const(right)?;

                match (op, l, r) {
                    (BinaryOp::Eq, ConstVal::Str(a), ConstVal::Str(b)) => {
                        Some(ConstVal::Bool(a == b))
                    }
                    (BinaryOp::Eq, ConstVal::Int(a), ConstVal::Int(b)) => {
                        Some(ConstVal::Bool(a == b))
                    }
                    (BinaryOp::Eq, ConstVal::Bool(a), ConstVal::Bool(b)) => {
                        Some(ConstVal::Bool(a == b))
                    }
                    (BinaryOp::Eq, ConstVal::Char(a), ConstVal::Char(b)) => {
                        Some(ConstVal::Bool(a == b))
                    }

                    (BinaryOp::NEq, ConstVal::Str(a), ConstVal::Str(b)) => {
                        Some(ConstVal::Bool(a != b))
                    }
                    (BinaryOp::NEq, ConstVal::Int(a), ConstVal::Int(b)) => {
                        Some(ConstVal::Bool(a != b))
                    }
                    (BinaryOp::NEq, ConstVal::Bool(a), ConstVal::Bool(b)) => {
                        Some(ConstVal::Bool(a != b))
                    }
                    (BinaryOp::NEq, ConstVal::Char(a), ConstVal::Char(b)) => {
                        Some(ConstVal::Bool(a != b))
                    }

                    (BinaryOp::And, ConstVal::Bool(a), ConstVal::Bool(b)) => {
                        Some(ConstVal::Bool(a && b))
                    }
                    (BinaryOp::Or, ConstVal::Bool(a), ConstVal::Bool(b)) => {
                        Some(ConstVal::Bool(a || b))
                    }

                    (BinaryOp::Gt, ConstVal::Int(a), ConstVal::Int(b)) => {
                        Some(ConstVal::Bool(a > b))
                    }
                    (BinaryOp::GtE, ConstVal::Int(a), ConstVal::Int(b)) => {
                        Some(ConstVal::Bool(a >= b))
                    }
                    (BinaryOp::Lt, ConstVal::Int(a), ConstVal::Int(b)) => {
                        Some(ConstVal::Bool(a < b))
                    }
                    (BinaryOp::LtE, ConstVal::Int(a), ConstVal::Int(b)) => {
                        Some(ConstVal::Bool(a <= b))
                    }
                    (BinaryOp::Add, ConstVal::Int(a), ConstVal::Int(b)) => {
                        Some(ConstVal::Int(a + b))
                    }
                    (BinaryOp::Sub, ConstVal::Int(a), ConstVal::Int(b)) => {
                        Some(ConstVal::Int(a - b))
                    }
                    (BinaryOp::Mul, ConstVal::Int(a), ConstVal::Int(b)) => {
                        Some(ConstVal::Int(a * b))
                    }
                    (BinaryOp::Div, ConstVal::Int(a), ConstVal::Int(b)) => {
                        if b == 0 {
                            None
                        } else {
                            Some(ConstVal::Int(a / b))
                        }
                    }
                    (BinaryOp::Mod, ConstVal::Int(a), ConstVal::Int(b)) => {
                        if b == 0 {
                            None
                        } else {
                            Some(ConstVal::Int(a % b))
                        }
                    }

                    _ => None,
                }
            }

            _ => None,
        }
    }

    pub fn type_of_expr(&self, expr: &Expr) -> Option<Type> {
        match &expr.kind {
            ExprKind::Literal(lit) => match lit {
                Literal::Int(_) => Some(Type::Int),
                Literal::String(_) => Some(Type::Str),
                Literal::Double(_) => Some(Type::Double),
                Literal::Float(_) => Some(Type::Float),
                Literal::Char(_) => Some(Type::Char),
                Literal::Bool(_) => Some(Type::Bool),
                Literal::Nil => Some(Type::Nil),
                Literal::Arr { elements } => {
                    let elem_type = elements.first().and_then(|e| self.type_of_expr(e))?;
                    Some(Type::Array {
                        element_type: Box::new(elem_type),
                        size: elements.len(),
                    })
                }
            },

            ExprKind::Identifier(name) => {
                let resolved = self.resolve_var_name(name);

                if let Some(ty) = self.var_types.get(&resolved) {
                    return Some(ty.clone());
                }

                if let Some(ty) = self.var_types.get(name) {
                    return Some(ty.clone());
                }

                None
            }

            ExprKind::Cast { right, .. } => Some(right.clone()),

            ExprKind::Binary { op, left, .. } => match op {
                BinaryOp::Eq
                | BinaryOp::NEq
                | BinaryOp::Gt
                | BinaryOp::GtE
                | BinaryOp::Lt
                | BinaryOp::LtE
                | BinaryOp::And
                | BinaryOp::Or => Some(Type::Bool),
                _ => self.type_of_expr(left),
            },

            ExprKind::Unary { op, expr } => match op {
                UnaryOp::Not => Some(Type::Bool),
                UnaryOp::AddressOf => {
                    let inner_ty = self.type_of_expr(expr)?;
                    Some(Type::Ptr(Box::new(inner_ty)))
                }
                UnaryOp::Deref => {
                    if let Some(Type::Ptr(inner_ty)) = self.type_of_expr(expr) {
                        Some(*inner_ty)
                    } else {
                        None
                    }
                }
                _ => self.type_of_expr(expr),
            },

            ExprKind::Typeof { .. } => Some(Type::Str),

            ExprKind::Field { base, field } => {
                if let ExprKind::Identifier(enum_name) = &base.kind
                    && self.enum_defs.contains_key(enum_name)
                {
                    return Some(Type::Enum(enum_name.clone()));
                }
                let base_ty = self.type_of_expr(base)?;

                let struct_name = match base_ty {
                    Type::Struct(name) => name,

                    Type::GenericInstance { name, args } => {
                        let mut mangled_name = name;

                        for arg in args {
                            mangled_name.push_str("__");
                            mangled_name.push_str(&self.mangle_type(&arg));
                        }

                        mangled_name
                    }

                    _ => return None,
                };

                self.struct_defs
                    .get(&struct_name)
                    .and_then(|layout| layout.field_offsets.get(field))
                    .map(|(_, ty)| ty.clone())
            }

            _ => None,
        }
    }

    pub fn next_temp_with_type(&mut self, ty: Type) -> String {
        let base_name = self.temps.next_temp();
        let qualified_name = if self.current_function.is_empty() {
            base_name
        } else {
            format!("{}::{}", self.current_function, base_name)
        };
        self.var_types.insert(qualified_name.clone(), ty);
        qualified_name
    }

    fn resolve_var_name(&self, name: &str) -> String {
        if let Some(aliased) = self.var_aliases.iter().rev().find_map(|s| s.get(name)) {
            return aliased.clone();
        }

        let local_mangled = format!("{}::{}", self.current_function, name);
        if self.var_types.get(&local_mangled).is_some() {
            return local_mangled;
        }

        name.to_string()
    }

    fn substitute_type(&self, ty: &Type, substitutions: &HashMap<String, Type>) -> Type {
        match ty {
            Type::Struct(name) => substitutions
                .get(name)
                .cloned()
                .unwrap_or(Type::Struct(name.clone())),

            Type::Ptr(inner) => Type::Ptr(Box::new(self.substitute_type(inner, substitutions))),

            Type::Array { element_type, size } => Type::Array {
                element_type: Box::new(self.substitute_type(element_type, substitutions)),
                size: *size,
            },

            Type::GenericInstance { name, args } => Type::GenericInstance {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|arg| self.substitute_type(arg, substitutions))
                    .collect(),
            },
            Type::GenericParam(name) => substitutions
                .get(name)
                .cloned()
                .unwrap_or_else(|| panic!("Unresolved generic parameter: {}", name)),

            Type::VariadicPack { .. } => {
                panic!(
                    "ICE: VariadicPack reached substitute_type: it should have been resolved to a concrete __variadic__ struct before codegen substitution."
                )
            }

            Type::Int
            | Type::UInt
            | Type::Int8
            | Type::UInt8
            | Type::Double
            | Type::Float
            | Type::Bool
            | Type::Str
            | Type::Char
            | Type::Void
            | Type::Nil
            | Type::Enum(..)
            | Type::Any => ty.clone(),
        }
    }

    fn mangle_type(&self, ty: &Type) -> String {
        crate::utils::typesafe::type_to_mangled_string(ty)
    }

    fn mangle_call_name(
        &self,
        base: &str,
        generic_args: &[Type],
        variadic_args: &[Type],
        is_variadic_capable: bool,
    ) -> String {
        let mut name = base.to_string();
        for arg in generic_args {
            name.push_str("__");
            name.push_str(&self.mangle_type(arg));
        }
        if is_variadic_capable {
            name.push('.');
            name.push_str(
                &variadic_args
                    .iter()
                    .map(|t| self.mangle_type(t))
                    .collect::<Vec<_>>()
                    .join("__"),
            );
        }
        name
    }

    /// Builds the layout for a variadic argument pack.
    ///
    /// This defers entirely to `typesafe::variadic::structure` — the same
    /// function the analyser uses to type-check field access on a
    /// `VariadicPack` (e.g. `pack.i0`, `pack.il`) — so the fields the
    /// analyser considers valid and the fields actually laid out in memory
    /// here can never drift apart. Field names/order come from that single
    /// source of truth instead of being duplicated as ad-hoc strings.
    fn instantiate_variadic_struct(
        &mut self,
        struct_name: &str,
        arg_types: &[Type],
        location: Location,
    ) {
        if self.struct_defs.contains_key(struct_name) {
            return;
        }

        let signature = variadic::structure(arg_types, location);

        let mut offset: i64 = 0;
        let mut max_align: i64 = 1;
        let mut field_offsets = IndexMap::new();

        for (field_name, ty) in signature.fields.iter() {
            let size = self.type_size(ty);
            let align = self.type_alignment(ty);
            if align > max_align {
                max_align = align;
            }
            offset = (offset + align - 1) & !(align - 1);
            field_offsets.insert(field_name.clone(), (offset, ty.clone()));
            offset += size;
        }

        let total_size = (offset + max_align - 1) & !(max_align - 1);

        self.struct_defs.insert(
            struct_name.to_string(),
            StructLayout {
                total_size,
                alignment: max_align,
                field_offsets,
            },
        );
    }

    pub fn resolve_type(&mut self, ty: &Type) -> Type {
        let substituted = if !self.current_substitutions.is_empty() {
            self.substitute_type(ty, &self.current_substitutions.clone())
        } else {
            ty.clone()
        };

        if substituted != *ty {
            return self.resolve_type(&substituted);
        }

        match substituted {
            Type::Struct(name) if self.enum_defs.contains_key(&name) => Type::Enum(name),

            Type::GenericInstance { name, args } => {
                let resolved_args: Vec<Type> =
                    args.iter().map(|arg| self.resolve_type(arg)).collect();

                let mut mangled_name = name.clone();
                for arg in &resolved_args {
                    mangled_name.push_str("__");
                    mangled_name.push_str(&self.mangle_type(arg));
                }

                if !self.struct_defs.contains_key(&mangled_name)
                    && let Some((params, fields)) = self.struct_blueprints.get(&name).cloned()
                {
                    let substitutions: HashMap<String, Type> =
                        params.into_iter().zip(resolved_args).collect();

                    self.instantiate_struct_layout(mangled_name.clone(), &fields, &substitutions);
                }
                Type::Struct(mangled_name)
            }
            Type::Ptr(inner) => Type::Ptr(Box::new(self.resolve_type(&inner))),
            Type::Array { element_type, size } => Type::Array {
                element_type: Box::new(self.resolve_type(&element_type)),
                size,
            },
            _ => substituted,
        }
    }

    fn instantiate_struct_layout(
        &mut self,
        mangled_name: String,
        fields: &[Parameter],
        substitutions: &HashMap<String, Type>,
    ) {
        let mut current_offset: i64 = 0;
        let mut max_alignment: i64 = 1;
        let mut field_offsets = IndexMap::new();

        for field in fields {
            let field_name = field.name.value.clone();
            let base_type = field.ptype.clone().unwrap_or(Type::Int);

            let substituted = self.substitute_type(&base_type, substitutions);
            let field_type = self.resolve_type(&substituted);

            let field_size = self.type_size(&field_type);
            let field_align = self.type_alignment(&field_type);

            if field_align > max_alignment {
                max_alignment = field_align;
            }

            current_offset = (current_offset + field_align - 1) & !(field_align - 1);
            field_offsets.insert(field_name, (current_offset, field_type));
            current_offset += field_size;
        }

        let total_size = (current_offset + max_alignment - 1) & !(max_alignment - 1);
        self.struct_defs.insert(
            mangled_name.clone(),
            StructLayout {
                total_size,
                alignment: max_alignment,
                field_offsets,
            },
        );
    }

    fn get_struct_layout(&self, name: &str) -> Option<&StructLayout> {
        if let Some(layout) = self.struct_defs.get(name) {
            return Some(layout);
        }
        if let Some(base_name) = name.split("__").next() {
            for (key, layout) in &self.struct_defs {
                if key == base_name || key.starts_with(&format!("{}__", base_name)) {
                    return Some(layout);
                }
            }
        }
        None
    }

    fn get_value_type(&self, value: &Value) -> Type {
        match value {
            Value::Temp(name) | Value::Var(name) => {
                self.var_types.get(name).cloned().unwrap_or(Type::Int)
            }
            Value::Const(_) => Type::Int,
            Value::Double(_) => Type::Double,
            Value::Float(_) => Type::Float,
            Value::Bool(_) => Type::Bool,
            Value::Char(_) => Type::Char,
            Value::Nil => Type::Nil,
            Value::Str(_) => Type::Str,
            Value::Void => Type::Void,
        }
    }

    fn type_size(&self, ty: &Type) -> i64 {
        match ty {
            Type::Enum(..) | Type::Int | Type::UInt | Type::Double => 8,
            Type::Float => 4,
            Type::Int8 | Type::UInt8 => 1,
            Type::Bool => 1,
            Type::Str => 8,
            Type::Ptr(_) => 8,
            Type::Array { element_type, size } => self.element_size(element_type) * (*size as i64),
            Type::GenericParam(name) => {
                panic!("Cannot get size of unresolved generic parameter: {}", name)
            }
            Type::Char => 1,
            Type::Struct(name) => self
                .get_struct_layout(name)
                .map(|l| l.total_size)
                .unwrap_or_else(|| panic!("Failed to find layout for struct: {name}")),
            Type::GenericInstance { name, args } => {
                let mut mangled_name = name.clone();
                for arg in args {
                    mangled_name.push_str("__");
                    mangled_name.push_str(&self.mangle_type(arg));
                }
                self.get_struct_layout(&mangled_name)
                    .map(|l| l.total_size)
                    .unwrap_or_else(|| {
                        panic!("Failed to find layout for generic instance: {mangled_name}")
                    })
            }
            Type::VariadicPack { .. } => {
                panic!(
                    "ICE: VariadicPack reached type_size: it should have been resolved to a concrete __variadic__ struct before size queries."
                )
            }

            Type::Void | Type::Nil => 0,
            Type::Any => 8, // default value, since any is unsafe anyway
        }
    }

    fn type_alignment(&self, ty: &Type) -> i64 {
        match ty {
            Type::Enum(..) | Type::Int | Type::UInt | Type::Double => 8,
            Type::Float => 4,
            Type::Int8 | Type::UInt8 => 1,
            Type::Bool => 1,
            Type::GenericParam(name) => {
                panic!(
                    "Cannot get alignment of unresolved generic parameter: {}",
                    name
                )
            }
            Type::Char => 1,
            Type::Str => 8,
            Type::Ptr(_) => 8,
            Type::Array { element_type, .. } => self.type_alignment(element_type),
            Type::Struct(name) => self
                .get_struct_layout(name)
                .map(|l| l.alignment)
                .unwrap_or_else(|| panic!("Failed to find layout for struct: {name}")),
            Type::GenericInstance { name, args } => {
                let mut mangled_name = name.clone();
                for arg in args {
                    mangled_name.push_str("__");
                    mangled_name.push_str(&self.mangle_type(arg));
                }
                self.get_struct_layout(&mangled_name)
                    .map(|l| l.alignment)
                    .unwrap_or_else(|| {
                        panic!("Failed to find layout for generic instance: {mangled_name}")
                    })
            }
            Type::VariadicPack { .. } => {
                panic!(
                    "ICE: VariadicPack reached type_alignment: it should have been resolved to a concrete __variadic__ struct before alignment queries."
                )
            }
            Type::Void | Type::Nil => 0,
            Type::Any => 8,
        }
    }

    fn element_size(&self, ty: &Type) -> i64 {
        self.type_size(ty)
    }

    fn emit_binary(&mut self, op: IrOp, lhs: Value, rhs: Value) -> Value {
        let lhs_ty = self.get_value_type(&lhs);
        let rhs_ty = self.get_value_type(&rhs);

        let result_ty = match op {
            IrOp::Add | IrOp::Sub | IrOp::Mul | IrOp::Div | IrOp::Mod => {
                if lhs_ty == Type::Str || rhs_ty == Type::Str {
                    Type::Str
                } else {
                    Type::Int
                }
            }
            IrOp::Eq | IrOp::NEq | IrOp::Gt | IrOp::GtE | IrOp::Lt | IrOp::LtE => Type::Bool,
            _ => Type::Int,
        };

        let temp = self.next_temp_with_type(result_ty);
        self.code.push(Instruction::Binary {
            dst: temp.clone(),
            op,
            lhs,
            rhs,
        });
        Value::Temp(temp)
    }

    fn emit_unary(&mut self, op: IrOp, value: Value) -> Value {
        let inner_ty = self.get_value_type(&value);

        let result_ty = match op {
            IrOp::Pos | IrOp::Neg => inner_ty,
            IrOp::Ref => Type::Ptr(Box::new(inner_ty)),
            _ => Type::Int,
        };

        let temp = self.next_temp_with_type(result_ty);
        self.code.push(Instruction::Unary {
            dst: temp.clone(),
            op,
            value,
        });
        Value::Temp(temp)
    }

    fn is_string_valued(&self, value: &Value) -> bool {
        matches!(value, Value::Str(_))
    }

    pub fn expr_type(&mut self, expr: &Expr) -> Option<Type> {
        match &expr.kind {
            ExprKind::Cast { left: _, right } => Some(right.clone()),
            ExprKind::Sizeof { .. } => Some(Type::Int),
            ExprKind::Typeof { .. } => Some(Type::Str),
            ExprKind::Literal(Literal::String(_)) => Some(Type::Str),
            ExprKind::Literal(Literal::Int(_)) => Some(Type::Int),
            ExprKind::Literal(Literal::Float(_)) => Some(Type::Float),
            ExprKind::Literal(Literal::Double(_)) => Some(Type::Double),
            ExprKind::Literal(Literal::Nil) => Some(Type::Nil),
            ExprKind::Literal(Literal::Bool(_)) => Some(Type::Bool),
            ExprKind::Literal(Literal::Char(_)) => Some(Type::Char),
            ExprKind::Literal(Literal::Arr { elements }) => {
                if !elements.is_empty() {
                    let element_type = self.expr_type(&elements[0])?;
                    Some(Type::Array {
                        element_type: Box::new(element_type),
                        size: elements.len(),
                    })
                } else {
                    Some(Type::Array {
                        element_type: Box::new(Type::Int),
                        size: 0,
                    })
                }
            }
            ExprKind::Identifier(name) => {
                let local_mangled = format!("{}::{}", self.current_function, name);
                if let Some(ty) = self.var_types.get(&local_mangled).cloned() {
                    return Some(self.resolve_type(&ty));
                }
                if let Some((ty, _)) = self.analyser_constants.get(name) {
                    let ty = ty.clone();
                    return Some(self.resolve_type(&ty));
                }
                if let Some(ty) = self.var_types.get(name).cloned() {
                    return Some(self.resolve_type(&ty));
                }
                None
            }
            ExprKind::Binary { left, op, .. } => match op {
                BinaryOp::Eq
                | BinaryOp::NEq
                | BinaryOp::Gt
                | BinaryOp::GtE
                | BinaryOp::Lt
                | BinaryOp::LtE => Some(Type::Bool),
                _ => self.expr_type(left),
            },
            ExprKind::Call { .. } => None,

            ExprKind::Index { base, .. } => match self.expr_type(base)? {
                Type::Array { element_type, .. } => Some(*element_type),
                Type::Str => Some(Type::Char),
                Type::Ptr(inner) => match *inner {
                    Type::Array { element_type, .. } => Some(*element_type),
                    other => Some(other),
                },
                _ => None,
            },

            ExprKind::Unary {
                op,
                expr: inner_expr,
            } => {
                let inner_type = self.expr_type(inner_expr)?;
                match op {
                    UnaryOp::AddressOf => Some(Type::Ptr(Box::new(inner_type))),
                    UnaryOp::Deref => match inner_type {
                        Type::Ptr(inner) => Some(*inner),
                        _ => None,
                    },
                    UnaryOp::Positive | UnaryOp::Negative => Some(Type::Int),
                    UnaryOp::Not => Some(Type::Bool),
                }
            }
            ExprKind::Field { base, field } => {
                if let Some(base_ty) = self.expr_type(base) {
                    let struct_name = match self.resolve_type(&base_ty) {
                        Type::Struct(name) => Some(name),
                        Type::GenericInstance { name, args } => {
                            let mut mangled_name = name;
                            for arg in args {
                                mangled_name.push_str("__");
                                mangled_name.push_str(&self.mangle_type(&arg));
                            }
                            Some(mangled_name)
                        }
                        _ => None,
                    };

                    if let Some(name) = struct_name {
                        let found_field_ty = self
                            .get_struct_layout(&name)
                            .and_then(|layout| layout.field_offsets.get(field))
                            .map(|(_, field_ty)| field_ty.clone());

                        if let Some(field_ty) = found_field_ty {
                            return Some(self.resolve_type(&field_ty));
                        }
                    }
                }
                None
            }
            ExprKind::StructLiteral { struct_name, .. } => Some(Type::Struct(struct_name.clone())),
        }
    }

    fn gen_call(
        &mut self,
        callee: &crate::parse::parsing::Identifier,
        generic_args: &[Type],
        args: &[Expr],
        want_result: bool,
    ) -> Option<Value> {
        let blueprint = self.fn_blueprints.get(&callee.value).cloned();

        let (generic_params, fixed_param_count, is_variadic_capable) =
            if let Some(Stmt::Function {
                generic_params,
                params,
                ..
            }) = &blueprint
            {
                let fixed = params.iter().filter(|p| !p.is_variadic).count();
                let variadic = params.iter().any(|p| p.is_variadic);
                (generic_params.clone(), fixed, variadic)
            } else {
                (Vec::new(), args.len(), false)
            };

        let substituted_generic_args: Vec<Type> = generic_args
            .iter()
            .map(|t| self.substitute_type(t, &self.current_substitutions))
            .collect();

        let split_at = fixed_param_count.min(args.len());
        let (fixed_arg_exprs, variadic_arg_exprs) = if is_variadic_capable {
            args.split_at(split_at)
        } else {
            (args, &args[args.len()..])
        };

        let mut arg_values: Vec<Value> = fixed_arg_exprs
            .iter()
            .map(|a| self.gen_expr(a, None))
            .collect();

        let mut variadic_types = Vec::new();
        let mut variadic_values = Vec::new();
        for a in variadic_arg_exprs {
            let v = self.gen_expr(a, None);
            let t = self.expr_type(a).unwrap_or(Type::Int);
            variadic_types.push(t);
            variadic_values.push(v);
        }

        let resolved_func_name = if blueprint.is_some() {
            self.mangle_call_name(
                &callee.value,
                &substituted_generic_args,
                &variadic_types,
                is_variadic_capable,
            )
        } else {
            callee.value.clone()
        };

        if is_variadic_capable {
            let struct_name = format!("__variadic__{}", resolved_func_name);
            self.instantiate_variadic_struct(
                &struct_name,
                &variadic_types,
                callee.location.clone(),
            );

            let raw = self.temps.next_temp();
            let pack_var = format!("_anon_struct_{}", raw);

            let pack_type = Type::Struct(struct_name.clone());

            self.var_types.insert(pack_var.clone(), pack_type.clone());

            let variadic_len = variadic_values.len() as i64;

            let store_field = |irgen: &mut Self, field_name: &str, val: Value| {
                let (offset, field_ty) =
                    irgen.struct_defs[&struct_name].field_offsets[field_name].clone();

                let base_addr_temp = irgen
                    .next_temp_with_type(Type::Ptr(Box::new(Type::Struct(struct_name.clone()))));
                irgen.code.push(Instruction::Unary {
                    dst: base_addr_temp.clone(),
                    op: IrOp::Ref,
                    value: Value::Var(pack_var.clone()),
                });

                let slot_addr_temp = irgen.next_temp_with_type(Type::Ptr(Box::new(field_ty)));
                irgen.code.push(Instruction::Binary {
                    dst: slot_addr_temp.clone(),
                    op: IrOp::Add,
                    lhs: Value::Temp(base_addr_temp),
                    rhs: Value::Const(offset),
                });

                irgen.code.push(Instruction::Store {
                    ptr: Value::Temp(slot_addr_temp),
                    source: val,
                });
            };

            for (i, val) in variadic_values.into_iter().enumerate() {
                store_field(self, &variadic::field_name(i), val);
            }
            store_field(self, variadic::length_field(), Value::Const(variadic_len));

            arg_values.push(Value::Var(pack_var));
        }

        for v in &arg_values {
            self.code.push(Instruction::Arg { value: v.clone() });
        }

        if blueprint.is_some() && !self.instantiated_fns.contains(&resolved_func_name) {
            self.instantiated_fns.insert(resolved_func_name.clone());
            self.deferred_instantiations.push((
                callee.value.clone(),
                substituted_generic_args.clone(),
                variadic_types.clone(),
            ));

            if let Some(Stmt::Function { rttype, .. }) = &blueprint {
                let substitutions: HashMap<String, Type> = generic_params
                    .iter()
                    .cloned()
                    .zip(substituted_generic_args.iter().cloned())
                    .collect();
                let unres_ty = rttype.clone().unwrap_or(Type::Void);
                let sub_ty = self.substitute_type(&unres_ty, &substitutions);

                let old_subs = self.current_substitutions.clone();
                self.current_substitutions = substitutions;
                let resolved_rttype = self.resolve_type(&sub_ty);
                self.current_substitutions = old_subs;

                self.var_types
                    .insert(resolved_func_name.clone(), resolved_rttype);
            }
        }

        let return_ty = self
            .var_types
            .get(&resolved_func_name)
            .cloned()
            .unwrap_or(Type::Int);

        if want_result {
            let dst = self.next_temp_with_type(return_ty);
            self.code.push(Instruction::Call {
                dest: Some(dst.clone()),
                name: resolved_func_name,
                argc: arg_values.len(),
            });
            Some(Value::Temp(dst))
        } else {
            self.code.push(Instruction::Call {
                dest: None,
                name: resolved_func_name,
                argc: arg_values.len(),
            });
            None
        }
    }

    fn gen_lvalue_addr(&mut self, expr: &Expr) -> Value {
        match &expr.kind {
            ExprKind::Identifier(name) => {
                let resolved_name = self.resolve_var_name(name);

                let ty = self
                    .var_types
                    .get(&resolved_name)
                    .cloned()
                    .unwrap_or(Type::Int);

                let temp = self.next_temp_with_type(Type::Ptr(Box::new(ty)));

                self.code.push(Instruction::Unary {
                    dst: temp.clone(),
                    op: IrOp::Ref,
                    value: Value::Var(resolved_name),
                });

                Value::Temp(temp)
            }

            ExprKind::Unary {
                op: UnaryOp::Deref,
                expr: inner,
            } => self.gen_expr(inner, None),

            ExprKind::Field { base, field } => {
                let base_addr = self.gen_lvalue_addr(base);

                let base_type = self.expr_type(base).unwrap_or(Type::Int);
                let resolved_base = self.resolve_type(&base_type);

                let struct_name = match resolved_base {
                    Type::Struct(name) => name,
                    Type::GenericInstance { name, args } => {
                        let mut mangled_name = name;
                        for arg in args {
                            mangled_name.push_str("__");
                            mangled_name.push_str(&self.mangle_type(&arg));
                        }
                        mangled_name
                    }
                    _ => panic!(
                        "Field access on non-struct type: {}",
                        type_to_string(&base_type)
                    ),
                };

                let (offset, field_type) = {
                    let (offset, unres_field_ty) = self
                        .struct_defs
                        .get(&struct_name)
                        .unwrap_or_else(|| panic!("Struct layout not found: {}", struct_name))
                        .field_offsets
                        .get(field)
                        .map(|(offset, field_ty)| (*offset, field_ty.clone()))
                        .unwrap_or_else(|| {
                            panic!("Field '{}' not found in struct '{}'", field, struct_name)
                        });

                    (offset, self.resolve_type(&unres_field_ty))
                };

                let field_addr_temp = self.next_temp_with_type(Type::Ptr(Box::new(field_type)));
                self.code.push(Instruction::Binary {
                    dst: field_addr_temp.clone(),
                    op: IrOp::Add,
                    lhs: base_addr,
                    rhs: Value::Const(offset),
                });

                Value::Temp(field_addr_temp)
            }

            ExprKind::Index { base, index } => {
                let base_addr = self.gen_lvalue_addr(base);
                let index_val = self.gen_expr(index, None);

                let base_type = self.expr_type(base);
                let element_type = match &base_type {
                    Some(Type::Array { element_type, .. }) => *element_type.clone(),
                    Some(Type::Ptr(inner)) => match &**inner {
                        Type::Array { element_type, .. } => *element_type.clone(),
                        other => other.clone(),
                    },
                    Some(Type::Str) => Type::Char,
                    _ => Type::Int,
                };

                let stride = self.element_size(&element_type);

                let offset_temp = self.next_temp_with_type(Type::Int);
                self.code.push(Instruction::Binary {
                    dst: offset_temp.clone(),
                    op: IrOp::Mul,
                    lhs: index_val,
                    rhs: Value::Const(stride),
                });

                let elem_addr_temp = self.next_temp_with_type(Type::Ptr(Box::new(element_type)));
                self.code.push(Instruction::Binary {
                    dst: elem_addr_temp.clone(),
                    op: IrOp::Add,
                    lhs: base_addr,
                    rhs: Value::Temp(offset_temp),
                });

                Value::Temp(elem_addr_temp)
            }

            _ => {
                panic!("Cannot take address of: {:?}", expr.kind);
            }
        }
    }

    pub fn gen_expr(&mut self, expr: &Expr, target_dest: Option<Value>) -> Value {
        match &expr.kind {
            ExprKind::Sizeof { ty } => {
                let resolved_ty = self.resolve_type(ty);
                let size = self.type_size(&resolved_ty);
                Value::Const(size)
            }
            ExprKind::Typeof { expr } => {
                let resolved_expr = self.expr_type(expr);
                if let Some(rexpr) = resolved_expr {
                    let etype = typesafe::typeof_string(&rexpr);
                    return Value::Str(etype);
                }

                panic!("ICE: typeof statement cannot resolve expression.")
            }

            ExprKind::Cast { left, right } => {
                let val_to_cast = self.gen_expr(left, None);

                let from_type = self.expr_type(left).unwrap_or(Type::Int);
                let to_type = self.resolve_type(right);

                let cast_kind = match (&from_type, &to_type) {
                    (Type::Ptr(_), Type::Ptr(_)) => CastType::BitCast,

                    (Type::Ptr(_), Type::Str) => CastType::BitCast,

                    (
                        Type::Int | Type::UInt | Type::Int8 | Type::UInt8,
                        Type::Int | Type::UInt | Type::Int8 | Type::UInt8,
                    ) => {
                        let from_size = self.type_size(&from_type);
                        let to_size = self.type_size(&to_type);

                        if from_size < to_size {
                            CastType::Extend
                        } else if from_size > to_size {
                            CastType::Truncate
                        } else {
                            CastType::BitCast
                        }
                    }

                    (Type::Float, Type::Double) => CastType::FloatExtend,

                    (Type::Double, Type::Float) => CastType::FloatTruncate,

                    (
                        Type::Int | Type::UInt | Type::Int8 | Type::UInt8,
                        Type::Float | Type::Double,
                    ) => CastType::IntToFloat,

                    (
                        Type::Float | Type::Double,
                        Type::Int | Type::UInt | Type::Int8 | Type::UInt8,
                    ) => CastType::FloatToInt,

                    _ => CastType::BitCast,
                };

                let result_temp = self.next_temp_with_type(to_type.clone());

                self.code.push(Instruction::Cast {
                    dst: result_temp.clone(),
                    cast_ty: cast_kind,
                    value: val_to_cast,
                    to_type,
                });

                Value::Temp(result_temp)
            }

            ExprKind::Literal(lit) => match lit {
                Literal::Int(v) => Value::Const(*v),
                Literal::Double(d) => Value::Double(*d),
                Literal::Float(f) => Value::Float(*f),
                Literal::String(s) => Value::Str(s.clone()),
                Literal::Bool(b) => Value::Bool(*b),
                Literal::Nil => Value::Nil,
                Literal::Char(c) => Value::Char(*c),
                Literal::Arr { elements } => {
                    let element_type = if !elements.is_empty() {
                        self.expr_type(&elements[0]).unwrap_or(Type::Int)
                    } else {
                        Type::Int
                    };
                    let stride = self.element_size(&element_type);

                    let base_val = match target_dest {
                        Some(dest) => dest,
                        None => {
                            let raw_temp = self.temps.next_temp();
                            let anon_name = format!("_anon_{}", raw_temp);
                            self.var_types.insert(
                                anon_name.clone(),
                                Type::Array {
                                    element_type: Box::new(element_type.clone()),
                                    size: elements.len(),
                                },
                            );
                            Value::Var(anon_name)
                        }
                    };

                    for (index, element_expr) in elements.iter().enumerate() {
                        let element_val = self.gen_expr(element_expr, None);

                        let offset_temp = self.next_temp_with_type(Type::Int);
                        self.code.push(Instruction::Binary {
                            dst: offset_temp.clone(),
                            op: IrOp::Mul,
                            lhs: Value::Const(index as i64),
                            rhs: Value::Const(stride),
                        });

                        let base_addr_temp =
                            self.next_temp_with_type(Type::Ptr(Box::new(element_type.clone())));
                        self.code.push(Instruction::Unary {
                            dst: base_addr_temp.clone(),
                            op: IrOp::Ref,
                            value: base_val.clone(),
                        });

                        let slot_addr_temp =
                            self.next_temp_with_type(Type::Ptr(Box::new(element_type.clone())));
                        self.code.push(Instruction::Binary {
                            dst: slot_addr_temp.clone(),
                            op: IrOp::Add,
                            lhs: Value::Temp(base_addr_temp),
                            rhs: Value::Temp(offset_temp),
                        });

                        self.code.push(Instruction::Store {
                            ptr: Value::Temp(slot_addr_temp),
                            source: element_val,
                        });
                    }

                    base_val
                }
            },

            ExprKind::Field { base, field } => {
                if let ExprKind::Identifier(enum_name) = &base.kind
                    && let Some(variants) = self.enum_defs.get(enum_name)
                {
                    let discriminant = *variants.get(field).unwrap_or_else(|| {
                        panic!("ICE: enum '{}' has no variant '{}'", enum_name, field)
                    });
                    return Value::Const(discriminant);
                }

                let base_addr = self.gen_lvalue_addr(base);
                let base_type = self.expr_type(base).unwrap_or(Type::Int);
                let resolved_base = self.resolve_type(&base_type);

                let struct_name = match resolved_base {
                    Type::Struct(name) => name,
                    Type::GenericInstance { name, args } => {
                        let mut mangled_name = name;
                        for arg in args {
                            mangled_name.push_str("__");
                            mangled_name.push_str(&self.mangle_type(&arg));
                        }
                        mangled_name
                    }
                    _ => panic!(
                        "ICE: Attempted field access on non-struct type. Found: {}",
                        type_to_string(&base_type)
                    ),
                };

                let (offset, field_type) = {
                    let (offset, unres_field_ty) = self
                        .get_struct_layout(&struct_name)
                        .unwrap_or_else(|| {
                            panic!(
                                "ICE: Structural reference layout untracked for '{}'.",
                                struct_name
                            )
                        })
                        .field_offsets
                        .get(field)
                        .map(|(offset, field_ty)| (*offset, field_ty.clone()))
                        .unwrap_or_else(|| {
                            panic!(
                                "ICE: Referenced struct field '{}' does not exist in '{}'.",
                                field, struct_name
                            )
                        });

                    (offset, self.resolve_type(&unres_field_ty))
                };

                let field_addr_temp =
                    self.next_temp_with_type(Type::Ptr(Box::new(field_type.clone())));
                self.code.push(Instruction::Binary {
                    dst: field_addr_temp.clone(),
                    op: IrOp::Add,
                    lhs: base_addr,
                    rhs: Value::Const(offset),
                });

                let result_temp = self.next_temp_with_type(field_type.clone());
                self.code.push(Instruction::Load {
                    dst: result_temp.clone(),
                    ptr: Value::Temp(field_addr_temp),
                    ty: field_type,
                });

                Value::Temp(result_temp)
            }

            ExprKind::StructLiteral {
                struct_name,
                generic_args,
                fields,
            } => {
                let concrete_type = if generic_args.is_empty() {
                    Type::Struct(struct_name.clone())
                } else {
                    let generic_ty = Type::GenericInstance {
                        name: struct_name.clone(),
                        args: generic_args.clone(),
                    };
                    self.resolve_type(&generic_ty)
                };

                let concrete_struct_name = match &concrete_type {
                    Type::Struct(name) => name.clone(),
                    _ => panic!("Expected concrete struct type after resolution"),
                };

                let target_val = match target_dest {
                    Some(dest) => dest,
                    None => {
                        let anon_name = format!("_anon_struct_{}", self.temps.next_temp());
                        self.var_types
                            .insert(anon_name.clone(), concrete_type.clone());

                        Value::Var(anon_name)
                    }
                };

                let layout_fields = self
                    .struct_defs
                    .get(&concrete_struct_name)
                    .expect("ICE: Structural initialization on untracked layout.")
                    .field_offsets
                    .clone();

                for (field_name, field_expr) in fields {
                    let field_val = self.gen_expr(field_expr, None);
                    let (offset, field_type) = layout_fields
                        .get(field_name)
                        .expect("ICE: Field initialization lookup failure.");

                    let base_addr_temp =
                        self.next_temp_with_type(Type::Ptr(Box::new(concrete_type.clone())));
                    self.code.push(Instruction::Unary {
                        dst: base_addr_temp.clone(),
                        op: IrOp::Ref,
                        value: target_val.clone(),
                    });

                    let slot_addr_temp =
                        self.next_temp_with_type(Type::Ptr(Box::new(field_type.clone())));
                    self.code.push(Instruction::Binary {
                        dst: slot_addr_temp.clone(),
                        op: IrOp::Add,
                        lhs: Value::Temp(base_addr_temp),
                        rhs: Value::Const(*offset),
                    });

                    self.code.push(Instruction::Store {
                        ptr: Value::Temp(slot_addr_temp),
                        source: field_val,
                    });
                }

                target_val
            }

            ExprKind::Index { base, index } => {
                let base_val = self.gen_expr(base, None);
                let index_val = self.gen_expr(index, None);

                let base_type = self.expr_type(base);
                let element_type = match &base_type {
                    Some(Type::Array { element_type, .. }) => *element_type.clone(),
                    Some(Type::Ptr(inner)) => match &**inner {
                        Type::Array { element_type, .. } => *element_type.clone(),
                        other => other.clone(),
                    },
                    Some(Type::Str) => Type::Char,
                    _ => Type::Int,
                };

                let stride = self.element_size(&element_type);
                let offset_temp = self.next_temp_with_type(Type::Int);
                self.code.push(Instruction::Binary {
                    dst: offset_temp.clone(),
                    op: IrOp::Mul,
                    lhs: index_val,
                    rhs: Value::Const(stride),
                });

                let target_addr_temp =
                    self.next_temp_with_type(Type::Ptr(Box::new(element_type.clone())));
                let is_base_variable_a_pointer = match &base_val {
                    Value::Var(name) => matches!(self.var_types.get(name), Some(Type::Ptr(_))),
                    _ => false,
                };

                if is_base_variable_a_pointer || matches!(base_type, Some(Type::Ptr(_))) {
                    self.code.push(Instruction::Binary {
                        dst: target_addr_temp.clone(),
                        op: IrOp::Add,
                        lhs: base_val,
                        rhs: Value::Temp(offset_temp),
                    });
                } else {
                    match base_val {
                        Value::Var(_) => {
                            let base_addr_temp =
                                self.next_temp_with_type(Type::Ptr(Box::new(element_type.clone())));
                            self.code.push(Instruction::Unary {
                                dst: base_addr_temp.clone(),
                                op: IrOp::Ref,
                                value: base_val,
                            });
                            self.code.push(Instruction::Binary {
                                dst: target_addr_temp.clone(),
                                op: IrOp::Add,
                                lhs: Value::Temp(base_addr_temp),
                                rhs: Value::Temp(offset_temp),
                            });
                        }
                        _ => {
                            self.code.push(Instruction::Binary {
                                dst: target_addr_temp.clone(),
                                op: IrOp::Add,
                                lhs: base_val,
                                rhs: Value::Temp(offset_temp),
                            });
                        }
                    }
                }

                let result_temp = self.next_temp_with_type(element_type.clone());
                self.code.push(Instruction::Load {
                    dst: result_temp.clone(),
                    ptr: Value::Temp(target_addr_temp),
                    ty: element_type,
                });

                Value::Temp(result_temp)
            }

            ExprKind::Identifier(name) => {
                let maybe_const_expr = self
                    .analyser_constants
                    .get(name)
                    .map(|(_, expr)| expr.clone());

                if let Some(expr) = maybe_const_expr {
                    if let Some(val) = self.evaluated_constants.get(name) {
                        return val.clone();
                    }
                    let val = self.gen_expr(&expr, None);
                    self.evaluated_constants.insert(name.clone(), val.clone());
                    return val;
                }

                Value::Var(self.resolve_var_name(name))
            }

            ExprKind::Unary { op, expr } => match op {
                UnaryOp::Positive => {
                    let value = self.gen_expr(expr, None);
                    self.emit_unary(IrOp::Pos, value)
                }
                UnaryOp::Negative => {
                    let value = self.gen_expr(expr, None);
                    self.emit_unary(IrOp::Neg, value)
                }
                UnaryOp::Deref => {
                    let value = self.gen_expr(expr, None);
                    let inner_type = self.expr_type(expr).unwrap_or(Type::Void);
                    let value_type = match inner_type {
                        Type::Ptr(inner) => *inner,
                        _ => {
                            unreachable!(
                                "non-pointer type dereferenced (this should be handled by analyser)"
                            )
                        }
                    };
                    let result_temp = self.next_temp_with_type(value_type.clone());
                    self.code.push(Instruction::Load {
                        dst: result_temp.clone(),
                        ptr: value,
                        ty: value_type,
                    });
                    Value::Temp(result_temp)
                }
                UnaryOp::Not => {
                    let value = self.gen_expr(expr, None);
                    self.emit_unary(IrOp::Not, value)
                }
                UnaryOp::AddressOf => {
                    if let ExprKind::Literal(lit) = &expr.kind {
                        let lit_val = match lit {
                            Literal::Int(v) => Value::Const(*v),
                            Literal::Double(d) => Value::Double(*d),
                            Literal::Float(f) => Value::Float(*f),
                            Literal::Nil => Value::Nil,
                            Literal::Bool(b) => Value::Bool(*b),
                            Literal::Char(c) => Value::Char(*c),
                            Literal::String(s) => Value::Str(s.clone()),
                            Literal::Arr { .. } => self.gen_expr(expr, None),
                        };

                        let lit_ty = self.expr_type(expr).unwrap_or(Type::Int);
                        let raw_temp = self.temps.next_temp();
                        let anon_var_name = format!("_anon_lit_{}", raw_temp);

                        self.var_types.insert(anon_var_name.clone(), lit_ty.clone());

                        self.code.push(Instruction::Assign {
                            dst: anon_var_name.clone(),
                            src: lit_val,
                        });

                        let ref_temp = self.next_temp_with_type(Type::Ptr(Box::new(lit_ty)));
                        self.code.push(Instruction::Unary {
                            dst: ref_temp.clone(),
                            op: IrOp::Ref,
                            value: Value::Var(anon_var_name),
                        });

                        Value::Temp(ref_temp)
                    } else if matches!(
                        expr.kind,
                        ExprKind::Field { .. }
                            | ExprKind::Index { .. }
                            | ExprKind::Unary {
                                op: UnaryOp::Deref,
                                ..
                            }
                    ) {
                        self.gen_lvalue_addr(expr)
                    } else {
                        let value = self.gen_expr(expr, None);
                        let inner_type = self.get_value_type(&value);

                        let raw_temp = self.temps.next_temp();
                        let anon_var_name = format!("_anon_ref_{}", raw_temp);
                        self.var_types
                            .insert(anon_var_name.clone(), inner_type.clone());
                        self.code.push(Instruction::Assign {
                            dst: anon_var_name.clone(),
                            src: value,
                        });

                        let temp = self.next_temp_with_type(Type::Ptr(Box::new(inner_type)));
                        self.code.push(Instruction::Unary {
                            dst: temp.clone(),
                            op: IrOp::Ref,
                            value: Value::Var(anon_var_name),
                        });
                        Value::Temp(temp)
                    }
                }
            },

            ExprKind::Binary { left, op, right } => {
                let lhs = self.gen_expr(left, None);
                let rhs = self.gen_expr(right, None);

                if matches!(op, BinaryOp::Add)
                    && (self.is_string_valued(&lhs) || self.expr_type(left) == Some(Type::Str))
                    && (self.is_string_valued(&rhs) || self.expr_type(right) == Some(Type::Str))
                {
                    self.code.push(Instruction::Arg { value: lhs });
                    self.code.push(Instruction::Arg { value: rhs });
                    let dst = self.next_temp_with_type(Type::Str);
                    self.code.push(Instruction::Call {
                        dest: Some(dst.clone()),
                        name: "str_concat".to_string(),
                        argc: 2,
                    });
                    return Value::Temp(dst);
                }

                let ir_op = match op {
                    BinaryOp::Add => IrOp::Add,
                    BinaryOp::Sub => IrOp::Sub,
                    BinaryOp::Mul => IrOp::Mul,
                    BinaryOp::Div => IrOp::Div,
                    BinaryOp::Eq => IrOp::Eq,
                    BinaryOp::NEq => IrOp::NEq,
                    BinaryOp::Gt => IrOp::Gt,
                    BinaryOp::GtE => IrOp::GtE,
                    BinaryOp::And => IrOp::And,
                    BinaryOp::Or => IrOp::Or,
                    BinaryOp::Lt => IrOp::Lt,
                    BinaryOp::LtE => IrOp::LtE,
                    BinaryOp::Mod => IrOp::Mod,
                };

                self.emit_binary(ir_op, lhs, rhs)
            }

            ExprKind::Call {
                callee,
                generic_args,
                args,
            } => self
                .gen_call(callee, generic_args, args, true)
                .unwrap_or(Value::Void),
        }
    }

    pub fn gen_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Use { .. } => unreachable!(),

            Stmt::Enum { name, options } => {
                let variants: IndexMap<String, i64> = options
                    .iter()
                    .enumerate()
                    .map(|(i, opt)| (opt.value.clone(), i as i64))
                    .collect();
                self.enum_defs.insert(name.value.clone(), variants);
            }

            Stmt::Struct {
                name,
                generic_params,
                fields,
            } => {
                if !generic_params.is_empty() {
                    self.struct_blueprints
                        .insert(name.value.clone(), (generic_params.clone(), fields.clone()));
                } else {
                    self.instantiate_struct_layout(name.value.clone(), fields, &HashMap::new());
                }
            }
            Stmt::Constant { .. } => {
                // Constants are generated at use sites
            }
            Stmt::Assignment { ident, vtype, expr } => {
                let mangled_name = format!("{}::{}", self.current_function, ident.value);

                if let Some(explicit_ty) = vtype {
                    let resolved = self.resolve_type(explicit_ty);
                    self.var_types.insert(mangled_name.clone(), resolved);
                }

                let current_ty = vtype
                    .clone()
                    .or_else(|| self.var_types.get(&mangled_name).cloned())
                    .map(|ty| self.resolve_type(&ty));

                let is_aggregate = matches!(
                    current_ty,
                    Some(Type::Array { .. })
                        | Some(Type::Struct(_))
                        | Some(Type::GenericInstance { .. })
                        | Some(Type::VariadicPack { .. })
                );

                let target_var = Value::Var(mangled_name.clone());

                if let Some(expr_node) = expr {
                    if is_aggregate {
                        let value = self.gen_expr(expr_node, Some(target_var));

                        self.code.push(Instruction::Assign {
                            dst: mangled_name,
                            src: value,
                        });
                    } else {
                        let value = self.gen_expr(expr_node, None);
                        if vtype.is_none() {
                            let computed_ty = self.get_value_type(&value);
                            let resolved_computed = self.resolve_type(&computed_ty);
                            self.var_types
                                .insert(mangled_name.clone(), resolved_computed);
                        }
                        self.code.push(Instruction::Assign {
                            dst: mangled_name,
                            src: value,
                        });
                    }
                } else {
                    match current_ty {
                        Some(Type::Int) => {
                            self.code.push(Instruction::Assign {
                                dst: mangled_name,
                                src: Value::Const(0),
                            });
                        }
                        Some(Type::Bool) => {
                            self.code.push(Instruction::Assign {
                                dst: mangled_name,
                                src: Value::Bool(false),
                            });
                        }
                        Some(Type::Char) => {
                            self.code.push(Instruction::Assign {
                                dst: mangled_name,
                                src: Value::Char('\0'),
                            });
                        }
                        Some(Type::Str) | Some(Type::Ptr(_)) => {
                            self.code.push(Instruction::Assign {
                                dst: mangled_name,
                                src: Value::Const(0),
                            });
                        }
                        Some(Type::Struct(_)) | Some(Type::Array { .. }) => {
                            self.code.push(Instruction::Assign {
                                dst: mangled_name,
                                src: Value::Const(0),
                            });
                        }
                        _ => {
                            self.code.push(Instruction::Assign {
                                dst: mangled_name,
                                src: Value::Const(0),
                            });
                        }
                    }
                }
            }

            Stmt::Reassignment { ident, expr } => {
                let mangled_name = format!("{}::{}", self.current_function, ident.value);
                let var_type = self.var_types.get(&mangled_name).cloned();

                let is_aggregate =
                    matches!(var_type, Some(Type::Array { .. } | Type::Struct { .. }));

                let target_var = Value::Var(mangled_name.clone());

                if is_aggregate {
                    let src_val = self.gen_expr(expr, Some(target_var.clone()));

                    if src_val != target_var {
                        self.code.push(Instruction::Store {
                            ptr: target_var,
                            source: src_val,
                        });
                    }
                } else {
                    let value = self.gen_expr(expr, None);
                    self.code.push(Instruction::Assign {
                        dst: mangled_name,
                        src: value,
                    });
                }
            }
            Stmt::Expr(expr) => {
                if let ExprKind::Call {
                    callee,
                    generic_args,
                    args,
                } = &expr.kind
                {
                    self.gen_call(callee, generic_args, args, false);
                } else {
                    self.gen_expr(expr, None);
                }
            }

            Stmt::If {
                cond,
                then_branch,
                else_if_branches,
                else_branch,
            } => {
                if let Some(ConstVal::Bool(is_true)) = self.eval_const(cond) {
                    if is_true {
                        for stmt in then_branch {
                            self.gen_stmt(stmt);
                        }
                        return;
                    }

                    let mut resolved_statically = true;
                    for (ei_cond, ei_body) in else_if_branches {
                        match self.eval_const(ei_cond) {
                            Some(ConstVal::Bool(true)) => {
                                for stmt in ei_body {
                                    self.gen_stmt(stmt);
                                }
                                return;
                            }
                            Some(ConstVal::Bool(false)) => continue,
                            _ => {
                                resolved_statically = false;
                                break;
                            }
                        }
                    }

                    if resolved_statically {
                        if let Some(else_stmts) = else_branch {
                            for stmt in else_stmts {
                                self.gen_stmt(stmt);
                            }
                        }
                        return;
                    }
                }

                let true_end = self.labels.next_label();
                let mut next_target = self.labels.next_label();

                let cond_val = self.gen_expr(cond, None);
                self.code.push(Instruction::JumpIfFalse {
                    cond: cond_val,
                    target: next_target.clone(),
                });

                for stmt in then_branch {
                    self.gen_stmt(stmt);
                }

                self.code.push(Instruction::Jump(true_end.clone()));

                for (ei_cond, ei_body) in else_if_branches.iter() {
                    self.code.push(Instruction::Label(next_target));

                    next_target = self.labels.next_label();

                    let ei_cond_val = self.gen_expr(ei_cond, None);
                    self.code.push(Instruction::JumpIfFalse {
                        cond: ei_cond_val,
                        target: next_target.clone(),
                    });

                    for stmt in ei_body {
                        self.gen_stmt(stmt);
                    }

                    self.code.push(Instruction::Jump(true_end.clone()));
                }

                if let Some(else_stmts) = else_branch {
                    self.code.push(Instruction::Label(next_target));
                    for stmt in else_stmts {
                        self.gen_stmt(stmt);
                    }
                } else if next_target != true_end {
                    self.code.push(Instruction::Label(next_target));
                }

                self.code.push(Instruction::Label(true_end));
            }
            Stmt::While { cond, body } => {
                let start = self.labels.next_label();
                let end = self.labels.next_label();

                self.loop_exits.push(end.clone());

                self.code.push(Instruction::Label(start.clone()));
                let cond_val = self.gen_expr(cond, None);
                self.code.push(Instruction::JumpIfFalse {
                    cond: cond_val,
                    target: end.clone(),
                });

                for stmt in body {
                    self.gen_stmt(stmt);
                }

                self.loop_exits.pop();
                self.code.push(Instruction::Jump(start));
                self.code.push(Instruction::Label(end));
            }
            Stmt::Break { .. } => {
                if let Some(exit_label) = self.loop_exits.last().cloned() {
                    self.code.push(Instruction::Jump(exit_label));
                } else {
                    panic!(
                        "Internal compiler error: break statement unvalidated by semantic analyzer"
                    );
                }
            }
            Stmt::ForIn {
                field_ident,
                target_expr,
                body,
            } => {
                let target_type = self
                    .expr_type(target_expr)
                    .unwrap_or_else(|| panic!("ICE: Cannot determine type of for-in target"));

                let resolved_type = self.resolve_type(&target_type);

                let target_value = self.gen_expr(target_expr, None);

                let field_var = format!("{}::{}", self.current_function, field_ident.value);

                match resolved_type {
                    Type::Struct(struct_name) => {
                        let layout = self.get_struct_layout(&struct_name).unwrap_or_else(|| {
                            panic!("ICE: Struct layout not found for '{}'", struct_name)
                        });

                        let mut fields: Vec<(i64, Type)> = layout
                            .field_offsets
                            .values()
                            .map(|(offset, ty)| (*offset, ty.clone()))
                            .collect();

                        fields.sort_by_key(|(offset, _)| *offset);

                        for (i, (offset, field_type)) in fields.into_iter().enumerate() {
                            let field_type = self.resolve_type(&field_type);

                            let iteration_var =
                                format!("{}::{}#{}", self.current_function, field_ident.value, i);
                            let shadow_var =
                                format!("{}::{}", self.current_function, field_ident.value);

                            let base_addr = self.next_temp_with_type(Type::Ptr(Box::new(
                                Type::Struct(struct_name.clone()),
                            )));

                            self.code.push(Instruction::Unary {
                                dst: base_addr.clone(),
                                op: IrOp::Ref,
                                value: target_value.clone(),
                            });

                            let field_addr =
                                self.next_temp_with_type(Type::Ptr(Box::new(field_type.clone())));

                            self.code.push(Instruction::Binary {
                                dst: field_addr.clone(),
                                op: IrOp::Add,
                                lhs: Value::Temp(base_addr),
                                rhs: Value::Const(offset),
                            });

                            let field_value = self.next_temp_with_type(field_type.clone());

                            self.code.push(Instruction::Load {
                                dst: field_value.clone(),
                                ptr: Value::Temp(field_addr),
                                ty: field_type.clone(),
                            });

                            self.var_types.push_scope();

                            let mut alias_scope = HashMap::new();
                            alias_scope.insert(field_ident.value.clone(), iteration_var.clone());
                            self.var_aliases.push(alias_scope);

                            self.var_types
                                .insert(iteration_var.clone(), field_type.clone());
                            self.var_types
                                .insert(shadow_var.clone(), field_type.clone());
                            self.var_types
                                .insert(field_ident.value.clone(), field_type.clone());

                            self.code.push(Instruction::Assign {
                                dst: iteration_var,
                                src: Value::Temp(field_value),
                            });

                            for stmt in body {
                                self.gen_stmt(stmt);
                            }

                            self.var_aliases.pop();
                            self.var_types.pop_scope();
                        }
                    }
                    Type::GenericInstance { name, args } => {
                        let concrete_type =
                            self.resolve_type(&Type::GenericInstance { name, args });

                        match concrete_type {
                            Type::Struct(struct_name) => {
                                let layout =
                                    self.get_struct_layout(&struct_name).unwrap_or_else(|| {
                                        panic!("ICE: Struct layout not found for '{}'", struct_name)
                                    });

                                let mut fields: Vec<(i64, Type)> = layout
                                    .field_offsets
                                    .values()
                                    .map(|(offset, ty)| (*offset, ty.clone()))
                                    .collect();

                                fields.sort_by_key(|(offset, _)| *offset);

                                for (offset, field_type) in fields {
                                    let field_type = self.resolve_type(&field_type);

                                    let base_addr = self.next_temp_with_type(Type::Ptr(Box::new(
                                        Type::Struct(struct_name.clone()),
                                    )));

                                    self.code.push(Instruction::Unary {
                                        dst: base_addr.clone(),
                                        op: IrOp::Ref,
                                        value: target_value.clone(),
                                    });

                                    let field_addr = self.next_temp_with_type(Type::Ptr(Box::new(
                                        field_type.clone(),
                                    )));

                                    self.code.push(Instruction::Binary {
                                        dst: field_addr.clone(),
                                        op: IrOp::Add,
                                        lhs: Value::Temp(base_addr),
                                        rhs: Value::Const(offset),
                                    });

                                    let field_value = self.next_temp_with_type(field_type.clone());

                                    self.code.push(Instruction::Load {
                                        dst: field_value.clone(),
                                        ptr: Value::Temp(field_addr),
                                        ty: field_type.clone(),
                                    });

                                    self.var_types.insert(field_var.clone(), field_type);

                                    self.code.push(Instruction::Assign {
                                        dst: field_var.clone(),
                                        src: Value::Temp(field_value),
                                    });

                                    for stmt in body {
                                        self.gen_stmt(stmt);
                                    }
                                }
                            }

                            other => {
                                panic!(
                                    "ICE: Generic for-in target resolved to non-struct type {}",
                                    type_to_string(&other)
                                );
                            }
                        }
                    }

                    Type::VariadicPack { .. } => {
                        /*
                         * A VariadicPack has already been materialised by gen_call()
                         * as a concrete __variadic__ struct. Therefore use its
                         * generated struct layout exactly like an ordinary struct.
                         */
                        let struct_type = self.resolve_type(&target_type);

                        let struct_name = match struct_type {
                            Type::Struct(name) => name,
                            other => {
                                panic!(
                                    "ICE: VariadicPack did not resolve to a struct: {}",
                                    type_to_string(&other)
                                );
                            }
                        };

                        let layout = self.get_struct_layout(&struct_name).unwrap_or_else(|| {
                            panic!("ICE: Variadic pack layout not found for '{}'", struct_name)
                        });

                        let mut fields: Vec<(String, i64, Type)> = layout
                            .field_offsets
                            .iter()
                            .map(|(name, (offset, ty))| (name.clone(), *offset, ty.clone()))
                            .filter(|(name, _, _)| name != variadic::length_field())
                            .collect();

                        fields.sort_by_key(|(_, offset, _)| *offset);

                        for (_, offset, field_type) in fields {
                            let field_type = self.resolve_type(&field_type);

                            let base_addr = self.next_temp_with_type(Type::Ptr(Box::new(
                                Type::Struct(struct_name.clone()),
                            )));

                            self.code.push(Instruction::Unary {
                                dst: base_addr.clone(),
                                op: IrOp::Ref,
                                value: target_value.clone(),
                            });

                            let field_addr =
                                self.next_temp_with_type(Type::Ptr(Box::new(field_type.clone())));

                            self.code.push(Instruction::Binary {
                                dst: field_addr.clone(),
                                op: IrOp::Add,
                                lhs: Value::Temp(base_addr),
                                rhs: Value::Const(offset),
                            });

                            let field_value = self.next_temp_with_type(field_type.clone());

                            self.code.push(Instruction::Load {
                                dst: field_value.clone(),
                                ptr: Value::Temp(field_addr),
                                ty: field_type.clone(),
                            });

                            self.var_types.insert(field_var.clone(), field_type);

                            self.code.push(Instruction::Assign {
                                dst: field_var.clone(),
                                src: Value::Temp(field_value),
                            });

                            for stmt in body {
                                self.gen_stmt(stmt);
                            }
                        }
                    }

                    other => {
                        panic!(
                            "ICE: Cannot use type {} as a for-in target",
                            type_to_string(&other)
                        );
                    }
                }
            }

            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                let start = self.labels.next_label();
                let end = self.labels.next_label();

                self.gen_stmt(init);
                self.code.push(Instruction::Label(start.clone()));
                let cond_val = self.gen_expr(cond, None);
                self.code.push(Instruction::JumpIfFalse {
                    cond: cond_val,
                    target: end.clone(),
                });

                for stmt in body {
                    self.gen_stmt(stmt);
                }
                self.gen_stmt(step);
                self.code.push(Instruction::Jump(start));
                self.code.push(Instruction::Label(end));
            }
            Stmt::Function {
                name,
                generic_params,
                params,
                body,
                rttype,
                ..
            } => {
                let has_variadic = params.iter().any(|p| p.is_variadic);
                if !generic_params.is_empty() || has_variadic {
                    self.fn_blueprints.insert(name.value.clone(), stmt.clone());
                    return;
                }

                let resolved_rttype = rttype
                    .clone()
                    .map(|ty| self.resolve_type(&ty))
                    .unwrap_or(Type::Void);
                self.var_types.insert(name.value.clone(), resolved_rttype);

                let start = self.functions.next(name.value.clone());
                let old_func = self.current_function.clone();
                self.current_function = start.clone();

                self.var_types.push_scope();

                self.code.push(Instruction::FunctionLabel(start.clone()));

                for param in params {
                    if let Some(param_ty) = &param.ptype {
                        let resolved_param_ty = self.resolve_type(param_ty);
                        let unique_param_name = format!("{}::{}", start, param.name.value);
                        self.var_types.insert(unique_param_name, resolved_param_ty);
                    }
                    self.code.push(Instruction::Param {
                        p: format!("{}::{}", start, param.name.value),
                    });
                }

                for stmt in body {
                    self.gen_stmt(stmt);
                }

                if !matches!(body.last(), Some(Stmt::Return { .. })) {
                    let fallback_val = Value::Void;
                    self.code.push(Instruction::Return {
                        value: fallback_val,
                    });
                }

                self.var_types.pop_scope();

                self.current_function = old_func;
            }
            Stmt::Return { value, .. } => {
                if let Some(expr) = value {
                    let val = self.gen_expr(expr, None);
                    self.code.push(Instruction::Return { value: val });
                } else {
                    self.code.push(Instruction::Return { value: Value::Void })
                }
            }
            Stmt::Extern { name, rttype, .. } => {
                let return_type = rttype.clone().unwrap_or(Type::Void);
                self.var_types.insert(name.value.clone(), return_type);
                self.code.push(Instruction::Extern {
                    fnname: name.value.clone(),
                });
            }
            Stmt::DerefReassignment { target, expr } => {
                let value_to_store = self.gen_expr(expr, None);

                match &target.kind {
                    ExprKind::Unary {
                        op: UnaryOp::Deref,
                        expr: inner,
                    } => {
                        let ptr_val = self.gen_expr(inner, None);
                        self.code.push(Instruction::Store {
                            ptr: ptr_val,
                            source: value_to_store,
                        });
                    }

                    ExprKind::Field { base, field } => {
                        let base_addr = self.gen_lvalue_addr(base);

                        let base_type = self.expr_type(base).unwrap_or(Type::Int);
                        let resolved_base = self.resolve_type(&base_type);

                        let struct_name = match resolved_base {
                            Type::Struct(name) => name,
                            Type::GenericInstance { name, args } => {
                                let mut mangled_name = name;
                                for arg in args {
                                    mangled_name.push_str("__");
                                    mangled_name.push_str(&self.mangle_type(&arg));
                                }
                                mangled_name
                            }
                            _ => panic!(
                                "ICE: Field assignment on non-struct type. Found: {}",
                                type_to_string(&base_type)
                            ),
                        };

                        let (offset, field_type) = {
                            let (offset, unres_field_ty) = self
                                .struct_defs
                                .get(&struct_name)
                                .unwrap_or_else(|| {
                                    panic!(
                                        "ICE: Structural reference layout untracked for '{}'.",
                                        struct_name
                                    )
                                })
                                .field_offsets
                                .get(field)
                                .map(|(offset, field_ty)| (*offset, field_ty.clone()))
                                .unwrap_or_else(|| {
                                    panic!(
                                        "ICE: Referenced struct field '{}' does not exist in '{}'.",
                                        field, struct_name
                                    )
                                });

                            (offset, self.resolve_type(&unres_field_ty))
                        };

                        let field_addr_temp =
                            self.next_temp_with_type(Type::Ptr(Box::new(field_type.clone())));
                        self.code.push(Instruction::Binary {
                            dst: field_addr_temp.clone(),
                            op: IrOp::Add,
                            lhs: base_addr,
                            rhs: Value::Const(offset),
                        });

                        self.code.push(Instruction::Store {
                            ptr: Value::Temp(field_addr_temp),
                            source: value_to_store,
                        });
                    }

                    ExprKind::Index { base, index } => {
                        let base_val = self.gen_expr(base, None);
                        let index_val = self.gen_expr(index, None);

                        let base_type = self.expr_type(base);
                        let element_type = match &base_type {
                            Some(Type::Array { element_type, .. }) => *element_type.clone(),
                            Some(Type::Ptr(inner)) => match &**inner {
                                Type::Array { element_type, .. } => *element_type.clone(),
                                other => other.clone(),
                            },
                            Some(Type::Str) => Type::Char,
                            _ => Type::Int,
                        };

                        let stride = self.element_size(&element_type);

                        let offset_temp = self.next_temp_with_type(Type::Int);
                        self.code.push(Instruction::Binary {
                            dst: offset_temp.clone(),
                            op: IrOp::Mul,
                            lhs: index_val,
                            rhs: Value::Const(stride),
                        });

                        let is_base_pointer = match &base.kind {
                            ExprKind::Identifier(name) => {
                                matches!(self.var_types.get(name), Some(Type::Ptr(_)))
                            }
                            ExprKind::Unary {
                                op: UnaryOp::Deref, ..
                            } => true,
                            _ => false,
                        };

                        let target_addr_temp =
                            self.next_temp_with_type(Type::Ptr(Box::new(element_type.clone())));

                        if is_base_pointer || matches!(base_type, Some(Type::Ptr(_))) {
                            self.code.push(Instruction::Binary {
                                dst: target_addr_temp.clone(),
                                op: IrOp::Add,
                                lhs: base_val,
                                rhs: Value::Temp(offset_temp),
                            });
                        } else {
                            let base_addr_temp =
                                self.next_temp_with_type(Type::Ptr(Box::new(element_type.clone())));
                            self.code.push(Instruction::Unary {
                                dst: base_addr_temp.clone(),
                                op: IrOp::Ref,
                                value: base_val,
                            });
                            self.code.push(Instruction::Binary {
                                dst: target_addr_temp.clone(),
                                op: IrOp::Add,
                                lhs: Value::Temp(base_addr_temp),
                                rhs: Value::Temp(offset_temp),
                            });
                        }

                        self.code.push(Instruction::Store {
                            ptr: Value::Temp(target_addr_temp),
                            source: value_to_store,
                        });
                    }

                    ExprKind::Identifier(name) => {
                        let dst = self.resolve_var_name(name);
                        self.code.push(Instruction::Assign {
                            dst,
                            src: value_to_store,
                        });
                    }

                    _ => {
                        panic!("Invalid lvalue in DerefReassignment: {:?}", target.kind);
                    }
                }
            }
        }
    }

    pub fn gen_param(&mut self, param: &Parameter) {
        self.code.push(Instruction::Param {
            p: param.name.value.clone(),
        });
    }

    pub fn gen_program(&mut self, program: &Program) {
        for stmt in &program.statements {
            if !matches!(stmt, Stmt::Function { .. })
                && !matches!(stmt, Stmt::Extern { .. })
                && !matches!(stmt, Stmt::Struct { .. })
                && !matches!(stmt, Stmt::Constant { .. })
                && !matches!(stmt, Stmt::Enum { .. })
            {
                println!(
                    "Codegen Error: top-level statement outside of a function is not supported."
                );
                std::process::exit(1);
            }
            self.gen_stmt(stmt);
        }

        while let Some((callee_name, generic_args, variadic_types)) =
            self.deferred_instantiations.pop()
        {
            if let Some(blueprint) = self.fn_blueprints.get(&callee_name).cloned()
                && let Stmt::Function {
                    name,
                    generic_params,
                    params,
                    body,
                    rttype,
                    ..
                } = blueprint
            {
                let has_variadic = params.iter().any(|p| p.is_variadic);
                let resolved_func_name = self.mangle_call_name(
                    &name.value,
                    &generic_args,
                    &variadic_types,
                    has_variadic,
                );

                let substitutions: HashMap<String, Type> = generic_params
                    .iter()
                    .cloned()
                    .zip(generic_args.iter().cloned())
                    .collect();

                let old_subs = self.current_substitutions.clone();
                self.current_substitutions = substitutions;

                let old_func = self.current_function.clone();
                self.current_function = resolved_func_name.clone();

                self.code
                    .push(Instruction::FunctionLabel(resolved_func_name.clone()));

                for param in params.iter().filter(|p| !p.is_variadic) {
                    if let Some(param_ty) = &param.ptype {
                        let resolved_param_ty = self.resolve_type(param_ty);
                        let unique_param_name =
                            format!("{}::{}", resolved_func_name, param.name.value);
                        self.var_types.insert(unique_param_name, resolved_param_ty);
                    }
                    self.code.push(Instruction::Param {
                        p: format!("{}::{}", resolved_func_name, param.name.value),
                    });
                }

                if let Some(variadic_param) = params.iter().find(|p| p.is_variadic) {
                    let struct_name = format!("__variadic__{}", resolved_func_name);
                    self.instantiate_variadic_struct(
                        &struct_name,
                        &variadic_types,
                        variadic_param.name.location.clone(),
                    );
                    let unique_param_name =
                        format!("{}::{}", resolved_func_name, variadic_param.name.value);
                    self.var_types
                        .insert(unique_param_name.clone(), Type::Struct(struct_name));
                    self.code.push(Instruction::Param {
                        p: unique_param_name,
                    });
                }

                for stmt in &body {
                    self.gen_stmt(stmt);
                }

                let base_return_ty = rttype.unwrap_or(Type::Void);
                let resolved_return_ty = self.resolve_type(&base_return_ty);

                if !matches!(self.code.last(), Some(Instruction::Return { .. })) {
                    let fallback_val = if resolved_return_ty == Type::Void {
                        Value::Void
                    } else if matches!(
                        resolved_return_ty,
                        Type::Struct(_) | Type::GenericInstance { .. }
                    ) {
                        let dummy_dst = self.next_temp_with_type(resolved_return_ty.clone());
                        Value::Temp(dummy_dst)
                    } else {
                        Value::Const(0)
                    };

                    self.code.push(Instruction::Return {
                        value: fallback_val,
                    });
                }

                self.current_function = old_func;
                self.current_substitutions = old_subs;
            }
        }
    }

    pub fn dump(&self) {
        for inst in &self.code {
            match inst {
                Instruction::Assign { dst, src } => println!("{dst} = {:?}", src),
                Instruction::Binary { dst, op, lhs, rhs } => {
                    println!("{dst} = {:?} {:?} {:?}", lhs, op, rhs)
                }
                Instruction::Unary { dst, op, value } => println!("{dst} = {:?}{:?}", op, value),
                Instruction::Label(label) => println!("{label}:"),
                Instruction::Jump(label) => println!("goto {label}"),
                Instruction::JumpIfFalse { cond, target } => {
                    println!("ifFalse {:?} goto {target}", cond)
                }
                Instruction::Param { p } => println!("param {}", p),
                Instruction::FunctionLabel(label) => println!("{label}:"),
                Instruction::Return { value } => println!("return {:?}", value),
                Instruction::Arg { value } => println!("arg {:?}", value),
                Instruction::Call { dest, name, argc } => println!(
                    "call {:?} @ {:?} [arg_count: {}]",
                    name,
                    dest.clone().unwrap_or("n/a".to_string()),
                    argc
                ),
                Instruction::Extern { fnname } => println!("extern {}", fnname),
                Instruction::Store { ptr, source } => println!("store {:?} to *{:?}", source, ptr),
                Instruction::Load { dst, ptr, ty } => {
                    println!("load {:?} [{:?}] from *{:?}", dst, ty, ptr)
                }
                Instruction::Cast {
                    dst,
                    cast_ty,
                    value,
                    to_type,
                } => println!(
                    "{dst} = {:?} as {:?} [casttype: {:?}]",
                    value, to_type, cast_ty
                ),
            }
        }
        println!("[DUMP_END]")
    }
}
impl Default for IRGen {
    fn default() -> Self {
        Self::new()
    }
}
