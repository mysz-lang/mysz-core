use std::collections::HashMap;

use crate::{
    ir::tac::{Instruction, IrOp, Value},
    parse::parsing::{BinaryOp, Expr, ExprKind, Literal, Parameter, Program, Stmt, Type, UnaryOp},
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

pub struct StructLayout {
    pub total_size: i64,
    pub field_offsets: HashMap<String, (i64, Type)>,
}

pub struct IRGen {
    pub code: Vec<Instruction>,
    temps: TempGen,
    labels: LabelGen,
    functions: FunctionGen,
    pub var_types: HashMap<String, Type>,
    pub struct_defs: HashMap<String, StructLayout>,
}

impl IRGen {
    pub fn new(types: HashMap<String, Type>) -> Self {
        Self {
            code: Vec::new(),
            temps: TempGen::new(),
            labels: LabelGen::new(),
            functions: FunctionGen::new(),
            struct_defs: HashMap::new(),
            var_types: types,
        }
    }

    fn next_temp_with_type(&mut self, ty: Type) -> String {
        let temp = self.temps.next();
        self.var_types.insert(temp.clone(), ty);
        temp
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
            Type::Int => 8,
            Type::Bool => 1,
            Type::Str => 8,
            Type::Ptr(_) => 8,
            Type::Array { element_type, size } => self.element_size(element_type) * (*size as i64),
            Type::Char => 1,
            Type::Struct(name) => {
                if let Some(layout) = self.struct_defs.get(name) {
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
            Type::Int => 8,
            Type::Bool => 1,
            Type::Char => 1,
            Type::Str => 8,
            Type::Ptr(_) => 8,
            Type::Array { element_type, .. } => self.type_alignment(element_type),
            Type::Struct(name) => {
                if let Some(layout) = self.struct_defs.get(name) {
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

    pub fn expr_type(&self, expr: &Expr) -> Option<Type> {
        match &expr.kind {
            ExprKind::Literal(Literal::String(_)) => Some(Type::Str),
            ExprKind::Literal(Literal::Int(_)) => Some(Type::Int),
            ExprKind::Literal(Literal::Bool(_)) => Some(Type::Bool),
            ExprKind::Literal(Literal::Char(_)) => Some(Type::Char),
            ExprKind::Identifier(name) => self.var_types.get(name).cloned(),
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
                }
            }

            ExprKind::Field { base, field } => {
                if let Some(Type::Struct(struct_name)) = self.expr_type(base) {
                    if let Some(layout) = self.struct_defs.get(&struct_name) {
                        if let Some((_, field_ty)) = layout.field_offsets.get(field) {
                            return Some(field_ty.clone());
                        }
                    }
                }
                None
            }
            ExprKind::StructLiteral { struct_name, .. } => Some(Type::Struct(struct_name.clone())),
            _ => None,
        }
    }

    pub fn gen_expr(&mut self, expr: &Expr, target_dest: Option<Value>) -> Value {
        match &expr.kind {
            ExprKind::Literal(lit) => match lit {
                Literal::Int(v) => Value::Const(*v),
                Literal::String(s) => Value::Str(s.clone()),
                Literal::Bool(b) => Value::Bool(*b),
                Literal::Char(c) => Value::Char(*c),
                Literal::Arr { elements } => {
                    let base_val = match target_dest {
                        Some(dest) => dest,
                        None => {
                            let raw_temp = self.temps.next();
                            let anon_name = format!("_anon_{}", raw_temp);
                            Value::Var(anon_name)
                        }
                    };

                    let element_type = if !elements.is_empty() {
                        self.expr_type(&elements[0]).unwrap_or(Type::Int)
                    } else {
                        Type::Int
                    };
                    let stride = self.element_size(&element_type);

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

                let (offset, field_type) = {
                    let base_type = self.expr_type(base).unwrap_or(Type::Int);
                    let struct_name = match base_type {
                        Type::Struct(name) => name,
                        _ => panic!(
                            "Internal Compiler Error: Attempted field access on a non-struct type."
                        ),
                    };

                    let layout = self
                        .struct_defs
                        .get(&struct_name)
                        .expect("Internal Compiler Error: Structural reference layout untracked.");
                    let (offset, field_ty) = layout
                        .field_offsets
                        .get(field)
                        .expect("Internal Compiler Error: Referenced struct field does not exist.");

                    (*offset, field_ty.clone())
                };

                let struct_name = match self.expr_type(base).unwrap_or(Type::Int) {
                    Type::Struct(name) => name,
                    _ => unreachable!(),
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
                // CHANGE #1: Unified allocation strategy
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
                    .expect(
                        "Internal Compiler Error: Structural initialization on untracked layout.",
                    )
                    .field_offsets
                    .clone();

                for (field_name, field_expr) in fields {
                    let field_val = self.gen_expr(field_expr, None);
                    let (offset, field_type) = layout_fields
                        .get(field_name)
                        .expect("Internal Compiler Error: Field initialization lookup failure.");

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

            ExprKind::Identifier(name) => Value::Var(name.clone()),

            ExprKind::Unary { op, expr } => {
                let value = self.gen_expr(expr, None);

                match op {
                    UnaryOp::Positive => self.emit_unary(IrOp::Pos, value),
                    UnaryOp::Negative => self.emit_unary(IrOp::Neg, value),
                    UnaryOp::Deref => {
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
                    UnaryOp::AddressOf => {
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

            ExprKind::Call { callee, args } => {
                let arg_values: Vec<Value> =
                    args.iter().map(|arg| self.gen_expr(arg, None)).collect();

                for val in arg_values.iter() {
                    self.code.push(Instruction::Arg { value: val.clone() });
                }

                let return_ty = self
                    .var_types
                    .get(&callee.value)
                    .cloned()
                    .unwrap_or(Type::Int);

                let dst = self.next_temp_with_type(return_ty);
                self.code.push(Instruction::Call {
                    dest: Some(dst.clone()),
                    name: callee.value.clone(),
                    argc: arg_values.len(),
                });

                Value::Temp(dst)
            }
        }
    }

    pub fn gen_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Use { .. } => unreachable!(),

            Stmt::Struct { name, fields } => {
                let mut current_offset: i64 = 0;
                let mut max_alignment: i64 = 1;
                let mut field_offsets = HashMap::new();

                for field in fields {
                    let field_name = field.name.value.clone();
                    let field_type = field.ptype.clone().unwrap_or(Type::Int);

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

                let layout = StructLayout {
                    total_size,
                    field_offsets,
                };

                self.struct_defs.insert(name.value.clone(), layout);
            }
            Stmt::Assignment { ident, vtype, expr } => {
                if let Some(explicit_ty) = vtype {
                    self.var_types
                        .insert(ident.value.clone(), explicit_ty.clone());
                }

                let current_ty = vtype
                    .clone()
                    .or_else(|| self.var_types.get(&ident.value).cloned());

                // CHANGE #2: Arrays are stored inline, but structs are copied seamlessly via standard assigns
                let is_array = match current_ty {
                    Some(Type::Array { .. }) => true,
                    _ => false,
                };

                let target_var = Value::Var(ident.value.clone());
                if is_array {
                    self.gen_expr(expr, Some(target_var));
                } else {
                    let value = self.gen_expr(expr, None);
                    if vtype.is_none() {
                        let computed_ty = self.get_value_type(&value);
                        self.var_types.insert(ident.value.clone(), computed_ty);
                    }
                    self.code.push(Instruction::Assign {
                        dst: ident.value.clone(),
                        src: value,
                    });
                }
            }
            Stmt::Reassignment { ident, expr } => {
                let value = self.gen_expr(expr, None);
                self.code.push(Instruction::Assign {
                    dst: ident.value.clone(),
                    src: value,
                });
            }
            Stmt::Expr(expr) => {
                if let ExprKind::Call { callee, args } = &expr.kind {
                    let arg_values: Vec<Value> =
                        args.iter().map(|arg| self.gen_expr(arg, None)).collect();

                    for val in arg_values.iter() {
                        self.code.push(Instruction::Arg { value: val.clone() });
                    }

                    self.code.push(Instruction::Call {
                        dest: None,
                        name: callee.value.clone(),
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

                self.code.push(Instruction::Label(start.clone()));
                let cond_val = self.gen_expr(cond, None);
                self.code.push(Instruction::JumpIfFalse {
                    cond: cond_val,
                    target: end.clone(),
                });

                for stmt in body {
                    self.gen_stmt(stmt);
                }
                self.code.push(Instruction::Jump(start));
                self.code.push(Instruction::Label(end));
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
                name, params, body, ..
            } => {
                let start = self.functions.next(name.value.clone());
                self.code.push(Instruction::FunctionLabel(start));

                for param in params {
                    if let Some(param_ty) = &param.ptype {
                        self.var_types
                            .insert(param.name.value.clone(), param_ty.clone());
                    }
                    self.gen_param(param);
                }

                for stmt in body {
                    self.gen_stmt(stmt);
                }

                if !matches!(body.last(), Some(Stmt::Return { .. })) {
                    self.code.push(Instruction::Return {
                        value: Value::Const(0),
                    });
                }
            }
            Stmt::Return { value, .. } => {
                if let Some(expr) = value {
                    let val = self.gen_expr(expr, None);
                    self.code.push(Instruction::Return { value: val });
                } else {
                    self.code.push(Instruction::Return {
                        value: Value::Const(0),
                    })
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
                            panic!(
                                "Internal Compiler Error: Attempted IR index calculation on non-indexable type."
                            );
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
                        let name = match base_type {
                            Type::Struct(n) => n,
                            _ => panic!(
                                "Internal Compiler Error: Field writing targeted a non-struct entity."
                            ),
                        };

                        let layout = self.struct_defs.get(&name).expect(
                            "Internal Compiler Error: Structural reference layout untracked.",
                        );
                        let (offset, field_ty) = layout.field_offsets.get(field).expect(
                            "Internal Compiler Error: Referenced struct field does not exist.",
                        );

                        (name, *offset, field_ty.clone())
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
            if !matches!(stmt, Stmt::Function { .. })
                && !matches!(stmt, Stmt::Extern { .. })
                && !matches!(stmt, Stmt::Struct { .. })
            {
                println!(
                    "Codegen Error: top-level statement outside of a function is not supported."
                );
                std::process::exit(1);
            }
            self.gen_stmt(stmt);
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