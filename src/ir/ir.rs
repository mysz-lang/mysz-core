use std::collections::HashMap;

use crate::{
    ir::tac::{Instruction, IrOp, ScopedMap, Value},
    parse::parsing::{BinaryOp, Expr, ExprKind, Literal, Parameter, Program, Stmt, Type, UnaryOp},
};

use crate::utils::typesafe::{
    is_integer, is_signed_integer, is_truthy_type, mangle_name, normalise_type, types_compatible,
};

pub struct TempGen {
    counter: usize,
}
impl TempGen {
    pub fn new() -> Self {
        Self { counter: 0 }
    }
    pub fn next(&mut self) -> String {
        self.counter += 1;
        format!("t{}", self.counter)
    }
}

pub struct LabelGen {
    counter: usize,
}
impl LabelGen {
    pub fn new() -> Self {
        Self { counter: 0 }
    }
    pub fn next(&mut self) -> String {
        self.counter += 1;
        format!("L{}", self.counter)
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

#[derive(Debug)]
pub struct StructLayout {
    pub total_size: i64,
    pub field_offsets: HashMap<String, (i64, Type)>,
}

pub struct IRGen {
    pub code: Vec<Instruction>,
    temps: TempGen,
    labels: LabelGen,
    functions: FunctionGen,
    loop_exits: Vec<String>,
    pub var_types: ScopedMap,
    pub struct_defs: HashMap<String, StructLayout>,
    pub struct_blueprints: HashMap<String, (Vec<String>, Vec<Parameter>)>,
    pub current_function: String,

    pub fn_blueprints: HashMap<String, Stmt>,
    pub instantiated_fns: std::collections::HashSet<String>,
    pub deferred_instantiations: Vec<(String, Vec<Type>)>,
    pub current_substitutions: HashMap<String, Type>,
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
            var_types: ScopedMap::new(HashMap::new()),
            current_function: String::new(),

            fn_blueprints: HashMap::new(),
            instantiated_fns: std::collections::HashSet::new(),
            deferred_instantiations: Vec::new(),
            current_substitutions: HashMap::new(),
        }
    }

    pub fn next_temp_with_type(&mut self, ty: Type) -> String {
        let base_name = self.temps.next();
        let qualified_name = if self.current_function.is_empty() {
            base_name
        } else {
            format!("{}::{}", self.current_function, base_name)
        };
        self.var_types.insert(qualified_name.clone(), ty);
        qualified_name
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

            Type::Int
            | Type::UInt
            | Type::Bool
            | Type::Str
            | Type::Char
            | Type::Void
            | Type::Any => ty.clone(),
        }
    }

    fn mangle_type(&self, ty: &Type) -> String {
        crate::utils::typesafe::type_to_mangled_string(ty)
    }

    pub fn resolve_type(&mut self, ty: &Type) -> Type {
        let substituted = if !self.current_substitutions.is_empty() {
            self.substitute_type(ty, &self.current_substitutions.clone())
        } else {
            ty.clone()
        };

        match &substituted {
            Type::GenericInstance { name, args } => {
                let resolved_args: Vec<Type> =
                    args.iter().map(|arg| self.resolve_type(arg)).collect();

                let mut mangled_name = name.clone();
                for arg in &resolved_args {
                    mangled_name.push_str("__");
                    mangled_name.push_str(&self.mangle_type(arg));
                }

                if !self.struct_defs.contains_key(&mangled_name) {
                    if let Some((params, fields)) = self.struct_blueprints.get(name).cloned() {
                        let substitutions: HashMap<String, Type> =
                            params.into_iter().zip(resolved_args.into_iter()).collect();

                        self.instantiate_struct_layout(
                            mangled_name.clone(),
                            &fields,
                            &substitutions,
                        );
                    }
                }
                Type::Struct(mangled_name)
            }
            Type::Ptr(inner) => Type::Ptr(Box::new(self.resolve_type(inner))),
            Type::Array { element_type, size } => Type::Array {
                element_type: Box::new(self.resolve_type(element_type)),
                size: *size,
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
        let mut field_offsets = HashMap::new();

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
            mangled_name,
            StructLayout {
                total_size,
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
            Value::Bool(_) => Type::Bool,
            Value::Char(_) => Type::Char,
            Value::Str(_) => Type::Str,
            Value::Void => Type::Void,
        }
    }

    fn type_size(&self, ty: &Type) -> i64 {
        match ty {
            Type::Int | Type::UInt => 8, // Explicitly handle UInt alongside Int!
            Type::Bool => 1,
            Type::Str => 8,
            Type::Ptr(_) => 8,
            Type::Array { element_type, size } => self.element_size(element_type) * (*size as i64),
            Type::Char => 1,
            Type::Struct(name) => {
                if let Some(layout) = self.get_struct_layout(name) {
                    layout.total_size
                } else {
                    8
                }
            }
            _ => 8,
        }
    }

    fn type_alignment(&self, ty: &Type) -> i64 {
        match ty {
            Type::Int | Type::UInt => 8, // Explicitly handle UInt alongside Int!
            Type::Bool => 1,
            Type::Char => 1,
            Type::Str => 8,
            Type::Ptr(_) => 8,
            Type::Array { element_type, .. } => self.type_alignment(element_type),
            Type::Struct(name) => {
                if let Some(layout) = self.get_struct_layout(name) {
                    layout
                        .field_offsets
                        .values()
                        .map(|(_, field_ty)| self.type_alignment(field_ty))
                        .max()
                        .unwrap_or(8)
                } else {
                    8
                }
            }
            _ => 8,
        }
    }

    fn element_size(&self, ty: &Type) -> i64 {
        match ty {
            Type::Bool => 1,
            Type::Char => 1,
            _ => self.type_size(ty),
        }
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
            ExprKind::Sizeof { .. } => Some(Type::Int),
            ExprKind::Literal(Literal::String(_)) => Some(Type::Str),
            ExprKind::Literal(Literal::Int(_)) => Some(Type::Int),
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
                let base_ty = if let Some(ty) = self.var_types.get(&local_mangled).cloned() {
                    ty
                } else {
                    self.var_types.get(name).cloned()?
                };
                Some(self.resolve_type(&base_ty))
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

    /// Computes the address of an lvalue-producing expression (a struct field
    /// access, an array/pointer index, or a pointer dereference) *without*
    /// first loading its value into a fresh temporary. This is what `&expr`
    /// must use for anything more complex than a bare identifier or literal --
    /// calling `gen_expr` and then `Ref`-ing the result instead gives you the
    /// address of a disconnected copy, silently breaking any write-through of
    /// that pointer (e.g. `&map.buckets` no longer points at `map`'s real
    /// `buckets` field).
    fn gen_lvalue_addr(&mut self, expr: &Expr) -> Value {
        match &expr.kind {
            ExprKind::Unary {
                op: UnaryOp::Deref,
                expr: inner,
            } => self.gen_expr(inner, None),

            ExprKind::Field { base, field } => {
                let base_addr = if matches!(
                    base.kind,
                    ExprKind::Field { .. }
                        | ExprKind::Index { .. }
                        | ExprKind::Unary {
                            op: UnaryOp::Deref,
                            ..
                        }
                ) {
                    self.gen_lvalue_addr(base)
                } else {
                    let base_val = self.gen_expr(base, None);
                    let base_ty = self.get_value_type(&base_val);
                    let addr_temp = self.next_temp_with_type(Type::Ptr(Box::new(base_ty)));
                    self.code.push(Instruction::Unary {
                        dst: addr_temp.clone(),
                        op: IrOp::Ref,
                        value: base_val,
                    });
                    Value::Temp(addr_temp)
                };

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
                        "ICE: Attempted field address on non-struct type. Found: {:?}",
                        base_type
                    ),
                };

                let (offset, field_type) = self
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
                let field_type = self.resolve_type(&field_type);

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
                let index_val = self.gen_expr(index, None);
                let base_type = self.expr_type(base);

                let element_type = match &base_type {
                    Some(Type::Array { element_type, .. }) => *element_type.clone(),
                    Some(Type::Ptr(inner)) => match &**inner {
                        Type::Array { element_type, .. } => *element_type.clone(),
                        other => other.clone(),
                    },
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

                let is_ptr_valued = matches!(base_type, Some(Type::Ptr(_)));
                let base_val =
                    if matches!(base.kind, ExprKind::Field { .. } | ExprKind::Index { .. })
                        && is_ptr_valued
                    {
                        self.gen_expr(base, None)
                    } else if matches!(
                        base.kind,
                        ExprKind::Unary {
                            op: UnaryOp::Deref,
                            ..
                        }
                    ) && is_ptr_valued
                    {
                        self.gen_expr(base, None)
                    } else {
                        self.gen_expr(base, None)
                    };

                let target_addr_temp =
                    self.next_temp_with_type(Type::Ptr(Box::new(element_type.clone())));

                if is_ptr_valued {
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

                Value::Temp(target_addr_temp)
            }

            _ => {
                let value = self.gen_expr(expr, None);
                let inner_type = self.get_value_type(&value);
                let temp = self.next_temp_with_type(Type::Ptr(Box::new(inner_type)));
                self.code.push(Instruction::Unary {
                    dst: temp.clone(),
                    op: IrOp::Ref,
                    value,
                });
                Value::Temp(temp)
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
            ExprKind::Literal(lit) => match lit {
                Literal::Int(v) => Value::Const(*v),
                Literal::String(s) => Value::Str(s.clone()),
                Literal::Bool(b) => Value::Bool(*b),
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
                            let raw_temp = self.temps.next();
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
                let base_val = self.gen_expr(base, None);
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
                        "ICE: Attempted field access on non-struct type. Found: {:?}",
                        base_type
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

                let base_addr_temp =
                    self.next_temp_with_type(Type::Ptr(Box::new(Type::Struct(struct_name))));
                self.code.push(Instruction::Unary {
                    dst: base_addr_temp.clone(),
                    op: IrOp::Ref,
                    value: base_val,
                });

                let field_addr_temp =
                    self.next_temp_with_type(Type::Ptr(Box::new(field_type.clone())));
                self.code.push(Instruction::Binary {
                    dst: field_addr_temp.clone(),
                    op: IrOp::Add,
                    lhs: Value::Temp(base_addr_temp),
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
                fields,
            } => {
                let target_val = match target_dest {
                    Some(dest) => dest,
                    None => {
                        let temp_name = self.next_temp_with_type(Type::Struct(struct_name.clone()));
                        Value::Temp(temp_name)
                    }
                };

                let layout_fields = self
                    .struct_defs
                    .get(struct_name)
                    .expect("ICE: Structural initialization on untracked layout.")
                    .field_offsets
                    .clone();

                for (field_name, field_expr) in fields {
                    let field_val = self.gen_expr(field_expr, None);
                    let (offset, field_type) = layout_fields
                        .get(field_name)
                        .expect("ICE: Field initialization lookup failure.");

                    let base_addr_temp = self.next_temp_with_type(Type::Ptr(Box::new(
                        Type::Struct(struct_name.clone()),
                    )));
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
                let local_mangled = format!("{}::{}", self.current_function, name);
                if self.var_types.get(&local_mangled).is_some() {
                    Value::Var(local_mangled)
                } else {
                    Value::Var(name.clone())
                }
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
                    let inner_type = self.expr_type(expr).unwrap_or(Type::Int);
                    let value_type = match inner_type {
                        Type::Ptr(inner) => *inner,
                        _ => Type::Int,
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
                            Literal::Bool(b) => Value::Bool(*b),
                            Literal::Char(c) => Value::Char(*c),
                            Literal::String(s) => Value::Str(s.clone()),
                            Literal::Arr { .. } => self.gen_expr(expr, None),
                        };

                        let lit_ty = self.expr_type(expr).unwrap_or(Type::Int);
                        let raw_temp = self.temps.next();
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
                        // These are lvalue-producing expressions: taking their address
                        // must compute a pointer to the *original* storage location
                        // (a struct field, an array/pointer element, or the pointee of
                        // a pointer) rather than evaluating the expression (which loads
                        // a copy of the value) and then taking the address of that copy.
                        self.gen_lvalue_addr(expr)
                    } else {
                        let value = self.gen_expr(expr, None);
                        let inner_type = self.get_value_type(&value);
                        let temp = self.next_temp_with_type(Type::Ptr(Box::new(inner_type)));
                        self.code.push(Instruction::Unary {
                            dst: temp.clone(),
                            op: IrOp::Ref,
                            value,
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
            } => {
                let arg_values: Vec<Value> =
                    args.iter().map(|arg| self.gen_expr(arg, None)).collect();

                for val in arg_values.iter() {
                    self.code.push(Instruction::Arg { value: val.clone() });
                }

                let mut resolved_func_name = callee.value.clone();
                let substituted_generic_args: Vec<Type> = generic_args
                    .iter()
                    .map(|arg_type| self.substitute_type(arg_type, &self.current_substitutions))
                    .collect();

                if !substituted_generic_args.is_empty() {
                    for arg_type in &substituted_generic_args {
                        resolved_func_name.push_str("__");
                        resolved_func_name.push_str(&self.mangle_type(arg_type));
                    }
                }

                if !substituted_generic_args.is_empty()
                    && !self.instantiated_fns.contains(&resolved_func_name)
                {
                    self.instantiated_fns.insert(resolved_func_name.clone());

                    // --- FIX: Push the substituted types so deferred instantiation works with concrete types! ---
                    self.deferred_instantiations
                        .push((callee.value.clone(), substituted_generic_args.clone()));

                    if let Some(Stmt::Function {
                        generic_params,
                        rttype,
                        ..
                    }) = self.fn_blueprints.get(&callee.value).cloned()
                    {
                        // --- FIX: Zip with substituted_generic_args ---
                        let substitutions: HashMap<String, Type> = generic_params
                            .iter()
                            .cloned()
                            .zip(substituted_generic_args.iter().cloned())
                            .collect();
                        let unres_ty = rttype.unwrap_or(Type::Void);
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

                let dst = self.next_temp_with_type(return_ty);
                self.code.push(Instruction::Call {
                    dest: Some(dst.clone()),
                    name: resolved_func_name,
                    argc: arg_values.len(),
                });

                Value::Temp(dst)
            }
        }
    }

    pub fn gen_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Use { .. } => unreachable!(),

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
            Stmt::Assignment { ident, vtype, expr } => {
                if let Some(explicit_ty) = vtype {
                    let resolved = self.resolve_type(explicit_ty);
                    self.var_types.insert(ident.value.clone(), resolved);
                }

                let current_ty = vtype
                    .clone()
                    .or_else(|| self.var_types.get(&ident.value).cloned())
                    .map(|ty| self.resolve_type(&ty));

                let is_array = match current_ty {
                    Some(Type::Array { .. }) => true,
                    _ => false,
                };

                let target_var = Value::Var(ident.value.clone());

                if let Some(expr_node) = expr {
                    if is_array {
                        self.gen_expr(expr_node, Some(target_var));
                    } else {
                        let value = self.gen_expr(expr_node, None);
                        if vtype.is_none() {
                            let computed_ty = self.get_value_type(&value);
                            let resolved_computed = self.resolve_type(&computed_ty);
                            self.var_types
                                .insert(ident.value.clone(), resolved_computed);
                        }
                        self.code.push(Instruction::Assign {
                            dst: ident.value.clone(),
                            src: value,
                        });
                    }
                } else {
                    match current_ty {
                        Some(Type::Int) => {
                            self.code.push(Instruction::Assign {
                                dst: ident.value.clone(),
                                src: Value::Const(0),
                            });
                        }
                        Some(Type::Bool) => {
                            self.code.push(Instruction::Assign {
                                dst: ident.value.clone(),
                                src: Value::Bool(false),
                            });
                        }
                        Some(Type::Char) => {
                            self.code.push(Instruction::Assign {
                                dst: ident.value.clone(),
                                src: Value::Char('\0'),
                            });
                        }
                        Some(Type::Str) | Some(Type::Ptr(_)) => {
                            self.code.push(Instruction::Assign {
                                dst: ident.value.clone(),
                                src: Value::Const(0),
                            });
                        }
                        Some(Type::Struct(_)) | Some(Type::Array { .. }) => {
                            self.code.push(Instruction::Assign {
                                dst: ident.value.clone(),
                                src: Value::Const(0),
                            });
                        }
                        _ => {
                            self.code.push(Instruction::Assign {
                                dst: ident.value.clone(),
                                src: Value::Const(0),
                            });
                        }
                    }
                }
            }
            Stmt::Reassignment { ident, expr } => {
                let is_array = matches!(self.var_types.get(&ident.value), Some(Type::Array { .. }));
                let target_var = Value::Var(ident.value.clone());

                if is_array {
                    self.gen_expr(expr, Some(target_var));
                } else {
                    let value = self.gen_expr(expr, None);
                    self.code.push(Instruction::Assign {
                        dst: ident.value.clone(),
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
                    let arg_values: Vec<Value> =
                        args.iter().map(|arg| self.gen_expr(arg, None)).collect();

                    for val in arg_values.iter() {
                        self.code.push(Instruction::Arg { value: val.clone() });
                    }

                    // --- FIX: Substitute generic arguments using active context ---
                    let mut resolved_func_name = callee.value.clone();
                    let substituted_generic_args: Vec<Type> = generic_args
                        .iter()
                        .map(|arg_type| self.substitute_type(arg_type, &self.current_substitutions))
                        .collect();

                    if !substituted_generic_args.is_empty() {
                        for arg_type in &substituted_generic_args {
                            resolved_func_name.push_str("__");
                            resolved_func_name.push_str(&self.mangle_type(arg_type));
                        }
                    }
                    // -------------------------------------------------------------

                    if !substituted_generic_args.is_empty()
                        && !self.instantiated_fns.contains(&resolved_func_name)
                    {
                        self.instantiated_fns.insert(resolved_func_name.clone());
                        self.deferred_instantiations
                            .push((callee.value.clone(), substituted_generic_args.clone())); // Push substituted types

                        if let Some(Stmt::Function {
                            generic_params,
                            rttype,
                            ..
                        }) = self.fn_blueprints.get(&callee.value).cloned()
                        {
                            let substitutions: HashMap<String, Type> = generic_params
                                .iter()
                                .cloned()
                                .zip(substituted_generic_args.iter().cloned()) // Use substituted types
                                .collect();
                            let unres_ty = rttype.unwrap_or(Type::Void);
                            let sub_ty = self.substitute_type(&unres_ty, &substitutions);

                            let old_subs = self.current_substitutions.clone();
                            self.current_substitutions = substitutions;
                            let resolved_rttype = self.resolve_type(&sub_ty);
                            self.current_substitutions = old_subs;

                            self.var_types
                                .insert(resolved_func_name.clone(), resolved_rttype);
                        }
                    }

                    self.code.push(Instruction::Call {
                        dest: None,
                        name: resolved_func_name,
                        argc: arg_values.len(),
                    });
                } else {
                    self.gen_expr(expr, None);
                }
            }
            Stmt::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let end = self.labels.next();
                let cond_val = self.gen_expr(cond, None);

                self.code.push(Instruction::JumpIfFalse {
                    cond: cond_val,
                    target: end.clone(),
                });

                for stmt in then_branch {
                    self.gen_stmt(stmt);
                }

                if else_branch.is_none() {
                    self.code.push(Instruction::Label(end));
                } else {
                    let true_end = self.labels.next();
                    self.code.push(Instruction::Jump(true_end.clone()));
                    self.code.push(Instruction::Label(end));
                    for stmt in else_branch.as_ref().unwrap() {
                        self.gen_stmt(stmt);
                    }
                    self.code.push(Instruction::Label(true_end));
                }
            }
            Stmt::While { cond, body } => {
                let start = self.labels.next();
                let end = self.labels.next();

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
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                let start = self.labels.next();
                let end = self.labels.next();

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
                ..
            } => {
                if !generic_params.is_empty() {
                    self.fn_blueprints.insert(name.value.clone(), stmt.clone());
                    return;
                }

                let start = self.functions.next(name.value.clone());
                let old_func = self.current_function.clone();
                self.current_function = start.clone();

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

                if let ExprKind::Index { base, index } = &target.kind {
                    let base_val = self.gen_expr(base, None);
                    let index_val = self.gen_expr(index, None);

                    let base_type = self.expr_type(base);
                    let element_type = match &base_type {
                        Some(Type::Array { element_type, .. }) => *element_type.clone(),
                        Some(Type::Ptr(inner)) => match &**inner {
                            Type::Array { element_type, .. } => *element_type.clone(),
                            other => other.clone(),
                        },
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
                    match base_type {
                        Some(Type::Array { .. }) => {
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
                        Some(Type::Ptr(_)) => {
                            self.code.push(Instruction::Binary {
                                dst: target_addr_temp.clone(),
                                op: IrOp::Add,
                                lhs: base_val,
                                rhs: Value::Temp(offset_temp),
                            });
                        }
                        _ => {
                            panic!("ICE: Attempted IR index calculation on non-indexable type.");
                        }
                    };

                    self.code.push(Instruction::Store {
                        ptr: Value::Temp(target_addr_temp),
                        source: value_to_store,
                    });
                } else if let ExprKind::Field { base, field } = &target.kind {
                    let base_val = self.gen_expr(base, None);

                    let (struct_name, offset, field_type) = {
                        let base_type = self.expr_type(base).unwrap_or(Type::Int);
                        let resolved_base = self.resolve_type(&base_type);

                        let name = match resolved_base {
                            Type::Struct(n) => n,
                            Type::GenericInstance { name, args } => {
                                let mut mangled_name = name;
                                for arg in args {
                                    mangled_name.push_str("__");
                                    mangled_name.push_str(&self.mangle_type(&arg));
                                }
                                mangled_name
                            }
                            _ => panic!(
                                "ICE: Field writing targeted a non-struct entity. Found: {:?}",
                                base_type
                            ),
                        };

                        let layout = self.struct_defs.get(&name).unwrap_or_else(|| {
                            panic!("ICE: Structural reference layout untracked for '{}'.", name)
                        });
                        let (offset, field_ty) =
                            layout.field_offsets.get(field).unwrap_or_else(|| {
                                panic!(
                                    "ICE: Referenced struct field '{}' does not exist in '{}'.",
                                    field, name
                                )
                            });

                        (name, *offset, self.resolve_type(&field_ty.clone()))
                    };

                    let base_addr_temp =
                        self.next_temp_with_type(Type::Ptr(Box::new(Type::Struct(struct_name))));
                    self.code.push(Instruction::Unary {
                        dst: base_addr_temp.clone(),
                        op: IrOp::Ref,
                        value: base_val,
                    });

                    let target_addr_temp =
                        self.next_temp_with_type(Type::Ptr(Box::new(field_type.clone())));
                    self.code.push(Instruction::Binary {
                        dst: target_addr_temp.clone(),
                        op: IrOp::Add,
                        lhs: Value::Temp(base_addr_temp),
                        rhs: Value::Const(offset),
                    });

                    self.code.push(Instruction::Store {
                        ptr: Value::Temp(target_addr_temp),
                        source: value_to_store,
                    });
                } else {
                    if let ExprKind::Unary {
                        op: crate::parse::parsing::UnaryOp::Deref,
                        expr: inner_expr,
                    } = &target.kind
                    {
                        let target_ptr_val = self.gen_expr(inner_expr, None);
                        self.code.push(Instruction::Store {
                            ptr: target_ptr_val,
                            source: value_to_store,
                        });
                    } else {
                        let target_ptr_val = self.gen_expr(target, None);
                        self.code.push(Instruction::Store {
                            ptr: target_ptr_val,
                            source: value_to_store,
                        });
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
            // stmts other than these outside of a function don't parse correctly so imma fix that soon, so you will be able to define statements outside of functions properly.
            // if !matches!(stmt, Stmt::Function { .. })
            //     && !matches!(stmt, Stmt::Extern { .. })
            //     && !matches!(stmt, Stmt::Struct { .. })
            // {
            //     println!(
            //         "Codegen Error: top-level statement outside of a function is not supported."
            //     );
            //     std::process::exit(1);
            // }
            self.gen_stmt(stmt);
        }

        while let Some((callee_name, args)) = self.deferred_instantiations.pop() {
            if let Some(blueprint) = self.fn_blueprints.get(&callee_name).cloned() {
                if let Stmt::Function {
                    name,
                    generic_params,
                    params,
                    body,
                    rttype,
                    ..
                } = blueprint
                {
                    let mut resolved_func_name = name.value.clone();
                    for arg_type in &args {
                        resolved_func_name.push_str("__");
                        resolved_func_name.push_str(&self.mangle_type(arg_type));
                    }

                    let substitutions: HashMap<String, Type> = generic_params
                        .iter()
                        .cloned()
                        .zip(args.iter().cloned())
                        .collect();

                    let old_subs = self.current_substitutions.clone();
                    self.current_substitutions = substitutions;

                    let old_func = self.current_function.clone();
                    self.current_function = resolved_func_name.clone();

                    self.code
                        .push(Instruction::FunctionLabel(resolved_func_name.clone()));

                    for param in params {
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

                    for stmt in body {
                        self.gen_stmt(&stmt);
                    }

                    let base_return_ty = rttype.unwrap_or(Type::Void);
                    let resolved_return_ty = self.resolve_type(&base_return_ty);

                    if !matches!(self.code.last(), Some(Instruction::Return { .. })) {
                        // --- CORRECTED WITH YOUR EXACT ENUM TYPES ---
                        let fallback_val = if resolved_return_ty == Type::Void {
                            Value::Void
                        } else if matches!(
                            resolved_return_ty,
                            Type::Struct(_) | Type::GenericInstance { .. }
                        ) {
                            // Struct structures and instantiated generic structures are aggregates!
                            // Generate a unique dummy temporary variable with this structure type
                            // layout so clback.rs handles it as a structured aggregate chunk copy.
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
            }
        }
    }
}
